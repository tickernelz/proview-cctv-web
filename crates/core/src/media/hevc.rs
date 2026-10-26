use super::rtp::RtpPacket;

pub const AP: u8 = 48;
pub const FU: u8 = 49;
pub const PACI: u8 = 50;

pub fn nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|byte| (byte >> 1) & 0x3f)
}

pub fn is_vcl(nal: &[u8]) -> bool {
    matches!(nal_type(nal), Some(kind) if kind <= 31)
}

pub fn is_keyframe(nal: &[u8]) -> bool {
    matches!(nal_type(nal), Some(16..=21))
}

pub fn append_annexb(nal: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&[0, 0, 0, 1]);
    out.extend_from_slice(nal);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit {
    pub nals: Vec<Vec<u8>>,
    pub keyframe: bool,
    pub pts_us: u64,
}

#[derive(Default)]
pub struct Assembler {
    nals: Vec<Vec<u8>>,
    keyframe: bool,
    timestamp: u32,
    expected: Option<u16>,
    epoch: u64,
    last: Option<u32>,
    in_sync: bool,
}

impl Assembler {
    pub fn new() -> Self {
        Self {
            in_sync: true,
            ..Self::default()
        }
    }

    pub fn synced(&self) -> bool {
        self.in_sync
    }

    pub fn packet(
        &mut self,
        sequence: u16,
        marker: bool,
        nals: &[Vec<u8>],
        timestamp: u32,
    ) -> Option<AccessUnit> {
        if let Some(expected) = self.expected {
            if expected != sequence {
                self.nals.clear();
                self.keyframe = false;
                self.in_sync = false;
            }
        }
        self.expected = Some(sequence.wrapping_add(1));
        if !self.in_sync {
            if nals.iter().any(|nal| is_keyframe(nal)) {
                self.in_sync = true;
            } else {
                return None;
            }
        }
        if self.nals.is_empty() {
            self.timestamp = timestamp;
        }
        for nal in nals {
            if is_keyframe(nal) {
                self.keyframe = true;
            }
            self.nals.push(nal.clone());
        }
        if marker {
            self.finish()
        } else {
            None
        }
    }

    fn finish(&mut self) -> Option<AccessUnit> {
        if self.nals.is_empty() {
            return None;
        }
        let nals = std::mem::take(&mut self.nals);
        let keyframe = std::mem::take(&mut self.keyframe);
        if let Some(last) = self.last {
            if self.timestamp.wrapping_sub(last) > u32::MAX / 2 {
                self.epoch += 1 << 32;
            }
        }
        self.last = Some(self.timestamp);
        let pts_us = ((self.timestamp as u64 + self.epoch) * 1_000_000) / 90_000;
        Some(AccessUnit {
            nals,
            keyframe,
            pts_us,
        })
    }
}

pub fn avcc(nals: &[Vec<u8>]) -> Vec<u8> {
    let capacity: usize = nals.iter().map(|nal| nal.len() + 4).sum();
    let mut out = Vec::with_capacity(capacity);
    for nal in nals {
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

pub fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nals {
        append_annexb(nal, &mut out);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileTierLevel {
    pub profile_space: u8,
    pub tier: bool,
    pub profile_idc: u8,
    pub compatibility: u32,
    pub constraints: u64,
    pub level_idc: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sps {
    pub profile: ProfileTierLevel,
    pub width: u32,
    pub height: u32,
    pub chroma_format_idc: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
}

struct Bits<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn u(&mut self, count: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            let byte = *self.data.get(self.bit / 8)?;
            let shift = 7 - (self.bit % 8);
            value = (value << 1) | u32::from((byte >> shift) & 1);
            self.bit += 1;
        }
        Some(value)
    }

    fn skip(&mut self, count: u32) -> Option<()> {
        self.u(count).map(|_| ())
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0u32;
        while self.u(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let rest = if zeros == 0 { 0 } else { self.u(zeros)? };
        Some(((1u32 << zeros) - 1) + rest)
    }
}

fn read_profile_tier_level(
    bits: &mut Bits<'_>,
    max_sub_layers: u32,
) -> Result<ProfileTierLevel, String> {
    let profile_space = bits.u(2).ok_or("sps ended inside the profile space")? as u8;
    let tier = bits.u(1).ok_or("sps ended inside the tier flag")? == 1;
    let profile_idc = bits.u(5).ok_or("sps ended inside the profile idc")? as u8;
    let compatibility = bits
        .u(32)
        .ok_or("sps ended inside the compatibility flags")?;
    let mut constraints = 0u64;
    for _ in 0..6 {
        constraints = (constraints << 8)
            | u64::from(bits.u(8).ok_or("sps ended inside the constraint flags")?);
    }
    let level_idc = bits.u(8).ok_or("sps ended inside the level idc")? as u8;

    let layers = max_sub_layers.min(7) as usize;
    let mut profile_present = [false; 8];
    let mut level_present = [false; 8];
    for index in 0..layers {
        profile_present[index] = bits.u(1).ok_or("sps ended inside the sub layer flags")? == 1;
        level_present[index] = bits.u(1).ok_or("sps ended inside the sub layer flags")? == 1;
    }
    if max_sub_layers > 0 {
        for _ in layers..8 {
            bits.skip(2)
                .ok_or("sps ended inside the reserved sub layer bits")?;
        }
    }
    for index in 0..layers {
        if profile_present[index] {
            bits.skip(88)
                .ok_or("sps ended inside a sub layer profile")?;
        }
        if level_present[index] {
            bits.skip(8).ok_or("sps ended inside a sub layer level")?;
        }
    }
    Ok(ProfileTierLevel {
        profile_space,
        tier,
        profile_idc,
        compatibility,
        constraints,
        level_idc,
    })
}

pub fn strip_emulation_prevention(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut index = 0;
    while index < payload.len() {
        if index + 2 < payload.len()
            && payload[index] == 0
            && payload[index + 1] == 0
            && payload[index + 2] == 3
        {
            out.extend_from_slice(&[0, 0]);
            index += 3;
        } else {
            out.push(payload[index]);
            index += 1;
        }
    }
    out
}

pub fn parse_sps(nal: &[u8]) -> Result<Sps, String> {
    if nal.len() < 6 {
        return Err("sps is too short".to_string());
    }
    let payload = strip_emulation_prevention(&nal[2..]);
    let mut bits = Bits::new(&payload);
    bits.skip(4)
        .ok_or("sps ended before the parameter set id")?;
    let max_sub_layers = bits.u(3).ok_or("sps ended before the sub layer count")?;
    bits.skip(1).ok_or("sps ended before the nesting flag")?;
    let profile = read_profile_tier_level(&mut bits, max_sub_layers)?;
    bits.ue().ok_or("sps ended before the sps id")?;
    let chroma_format_idc = bits.ue().ok_or("sps ended before the chroma format")?;
    if chroma_format_idc == 3 {
        bits.skip(1)
            .ok_or("sps ended before the colour plane flag")?;
    }
    let mut width = bits.ue().ok_or("sps ended before the width")?;
    let mut height = bits.ue().ok_or("sps ended before the height")?;
    let conformance = bits.u(1).ok_or("sps ended before the conformance flag")?;
    if conformance == 1 {
        let left = bits.ue().ok_or("sps ended inside the conformance window")?;
        let right = bits.ue().ok_or("sps ended inside the conformance window")?;
        let top = bits.ue().ok_or("sps ended inside the conformance window")?;
        let bottom = bits.ue().ok_or("sps ended inside the conformance window")?;
        let unit = if chroma_format_idc == 1 || chroma_format_idc == 2 {
            2
        } else {
            1
        };
        width = width.saturating_sub(unit * (left + right));
        height = height.saturating_sub(unit * (top + bottom));
    }
    let bit_depth_luma = bits.ue().ok_or("sps ended before the luma depth")? as u8 + 8;
    let bit_depth_chroma = bits.ue().ok_or("sps ended before the chroma depth")? as u8 + 8;
    Ok(Sps {
        profile,
        width,
        height,
        chroma_format_idc: chroma_format_idc as u8,
        bit_depth_luma,
        bit_depth_chroma,
    })
}

pub fn codec_string(profile: &ProfileTierLevel) -> String {
    let space = match profile.profile_space {
        1 => "A",
        2 => "B",
        3 => "C",
        _ => "",
    };
    let compatibility = profile.compatibility.reverse_bits();
    let tier = if profile.tier { 'H' } else { 'L' };
    let mut out = format!(
        "hvc1.{space}{}.{compatibility:x}.{tier}{}",
        profile.profile_idc, profile.level_idc
    );
    let bytes = profile.constraints.to_be_bytes();
    let mut end = bytes.len();
    while end > 2 && bytes[end - 1] == 0 {
        end -= 1;
    }
    if end > 2 {
        out.push('.');
        for byte in &bytes[2..end] {
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

pub fn build_hvcc(sps: &Sps, vps: &[u8], sps_nal: &[u8], pps: &[u8]) -> Vec<u8> {
    let profile = sps.profile;
    let mut out = Vec::with_capacity(23 + vps.len() + sps_nal.len() + pps.len() + 18);
    out.push(1);
    out.push(
        (profile.profile_space << 6) | (u8::from(profile.tier) << 5) | (profile.profile_idc & 0x1f),
    );
    out.extend_from_slice(&profile.compatibility.to_be_bytes());
    out.extend_from_slice(&profile.constraints.to_be_bytes()[2..]);
    out.push(profile.level_idc);
    out.extend_from_slice(&[0xf0, 0x00]);
    out.push(0xfc);
    out.push(0xfc | (sps.chroma_format_idc & 0x03));
    out.push(0xf8 | (sps.bit_depth_luma.saturating_sub(8) & 0x07));
    out.push(0xf8 | (sps.bit_depth_chroma.saturating_sub(8) & 0x07));
    out.extend_from_slice(&0u16.to_be_bytes());
    out.push(0x0f);
    out.push(3);
    for (kind, unit) in [(32u8, vps), (33, sps_nal), (34, pps)] {
        out.push(0x80 | kind);
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&(unit.len() as u16).to_be_bytes());
        out.extend_from_slice(unit);
    }
    out
}

struct Fragment {
    nal: Vec<u8>,
}

#[derive(Default)]
pub struct Depacketizer {
    fragment: Option<Fragment>,
    dropped: u64,
}

impl Depacketizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn push(&mut self, packet: &RtpPacket<'_>) -> Vec<Vec<u8>> {
        let payload = packet.payload;
        if payload.len() < 2 {
            self.discard();
            return Vec::new();
        }
        if payload[0] & 0x80 != 0 {
            self.discard();
            return Vec::new();
        }
        match (payload[0] >> 1) & 0x3f {
            AP => self.aggregation(&payload[2..]),
            FU => self.fragmentation(payload),
            PACI => Vec::new(),
            _ => {
                self.discard();
                vec![payload.to_vec()]
            }
        }
    }

    fn discard(&mut self) {
        if self.fragment.take().is_some() {
            self.dropped += 1;
        }
    }

    fn aggregation(&mut self, body: &[u8]) -> Vec<Vec<u8>> {
        let mut units = Vec::new();
        let mut offset = 0;
        while offset + 2 <= body.len() {
            let size = u16::from_be_bytes([body[offset], body[offset + 1]]) as usize;
            offset += 2;
            if size == 0 || offset + size > body.len() {
                break;
            }
            units.push(body[offset..offset + size].to_vec());
            offset += size;
        }
        units
    }

    fn fragmentation(&mut self, payload: &[u8]) -> Vec<Vec<u8>> {
        if payload.len() < 3 {
            self.discard();
            return Vec::new();
        }
        let header = payload[2];
        let start = header & 0x80 != 0;
        let end = header & 0x40 != 0;
        let kind = header & 0x3f;
        if start {
            self.discard();
            self.fragment = Some(Fragment {
                nal: vec![(payload[0] & 0x81) | (kind << 1), payload[1]],
            });
            if let Some(fragment) = self.fragment.as_mut() {
                fragment.nal.extend_from_slice(&payload[3..]);
            }
            return if end { self.take() } else { Vec::new() };
        }
        match self.fragment.as_mut() {
            Some(fragment) => {
                fragment.nal.extend_from_slice(&payload[3..]);
                if end {
                    self.take()
                } else {
                    Vec::new()
                }
            }
            None => {
                self.dropped += 1;
                Vec::new()
            }
        }
    }

    fn take(&mut self) -> Vec<Vec<u8>> {
        match self.fragment.take() {
            Some(fragment) => vec![fragment.nal],
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::rtp;

    fn packet(marker: bool, body: &[u8]) -> Vec<u8> {
        let mut raw = vec![0x80, if marker { 0x80 } else { 0 } | 98];
        raw.extend_from_slice(&1u16.to_be_bytes());
        raw.extend_from_slice(&0u32.to_be_bytes());
        raw.extend_from_slice(&1u32.to_be_bytes());
        raw.extend_from_slice(body);
        raw
    }

    fn feed(depacketizer: &mut Depacketizer, marker: bool, body: &[u8]) -> Vec<Vec<u8>> {
        let raw = packet(marker, body);
        let parsed = rtp::parse(&raw).expect("packet");
        depacketizer.push(&parsed)
    }

    #[test]
    fn a_single_nal_unit_passes_through_unchanged() {
        let mut depacketizer = Depacketizer::new();
        let nal = [0x26, 0x01, 0xaf, 0x10];
        assert_eq!(feed(&mut depacketizer, true, &nal), vec![nal.to_vec()]);
        assert_eq!(nal_type(&nal), Some(19));
        assert!(is_keyframe(&nal));
        assert!(is_vcl(&nal));
    }

    #[test]
    fn three_fragments_reassemble_into_one_nal_unit() {
        let mut depacketizer = Depacketizer::new();
        let start = [0x62, 0x01, 0x93, 0xaa, 0xbb];
        let middle = [0x62, 0x01, 0x13, 0xcc];
        let end = [0x62, 0x01, 0x53, 0xdd, 0xee];
        assert!(feed(&mut depacketizer, false, &start).is_empty());
        assert!(feed(&mut depacketizer, false, &middle).is_empty());
        let units = feed(&mut depacketizer, true, &end);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0], vec![0x26, 0x01, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
        assert!(is_keyframe(&units[0]));
        assert_eq!(depacketizer.dropped(), 0);
    }

    #[test]
    fn an_orphan_fragment_is_dropped_without_panicking() {
        let mut depacketizer = Depacketizer::new();
        let middle = [0x62, 0x01, 0x13, 0xcc];
        assert!(feed(&mut depacketizer, false, &middle).is_empty());
        assert_eq!(depacketizer.dropped(), 1);
    }

    #[test]
    fn an_aggregation_packet_yields_each_nal_unit() {
        let mut depacketizer = Depacketizer::new();
        let first = [0x40, 0x01, 0x0c, 0x01];
        let second = [0x42, 0x01, 0x01, 0x60];
        let mut body = vec![0x60, 0x01];
        body.extend_from_slice(&(first.len() as u16).to_be_bytes());
        body.extend_from_slice(&first);
        body.extend_from_slice(&(second.len() as u16).to_be_bytes());
        body.extend_from_slice(&second);
        let units = feed(&mut depacketizer, false, &body);
        assert_eq!(units, vec![first.to_vec(), second.to_vec()]);
    }

    #[test]
    fn a_corrupt_fu_header_flag_is_dropped() {
        let mut depacketizer = Depacketizer::new();
        let corrupt = [0xe2, 0x01, 0x93, 0xaa];
        assert!(feed(&mut depacketizer, false, &corrupt).is_empty());
        assert_eq!(depacketizer.dropped(), 0);
    }

    fn profile() -> ProfileTierLevel {
        ProfileTierLevel {
            profile_space: 0,
            tier: false,
            profile_idc: 1,
            compatibility: 0x6000_0000,
            constraints: 0x9000_0000_0000,
            level_idc: 120,
        }
    }

    fn sps_bytes() -> Vec<u8> {
        let mut sps = vec![0x42, 0x01];
        sps.push(0x01);
        sps.push(0x01);
        sps.extend_from_slice(&0x6000_0000u32.to_be_bytes());
        sps.extend_from_slice(&[0x90, 0x00, 0x00, 0x00, 0x00, 0x00]);
        sps.push(120);
        sps.push(0x00);
        sps.push(0x00);
        sps.push(0x00);
        sps
    }

    #[test]
    fn the_codec_string_follows_the_iso_tier_and_level_spelling() {
        assert_eq!(codec_string(&profile()), "hvc1.1.6.L120.90");
        let bare = ProfileTierLevel {
            constraints: 0,
            level_idc: 63,
            ..profile()
        };
        assert_eq!(codec_string(&bare), "hvc1.1.6.L63");
    }

    #[test]
    fn emulation_prevention_bytes_are_removed_before_parsing() {
        let raw = [0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x00, 0x02];
        assert_eq!(
            strip_emulation_prevention(&raw),
            vec![0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02]
        );
    }

    #[test]
    fn hvcc_carries_the_profile_and_one_array_per_parameter_set() {
        let sps_nal = sps_bytes();
        let hvcc = build_hvcc(
            &Sps {
                profile: profile(),
                width: 2304,
                height: 2592,
                chroma_format_idc: 1,
                bit_depth_luma: 8,
                bit_depth_chroma: 8,
            },
            &[0x40, 0x01, 0x0c, 0x01],
            &sps_nal,
            &[0x44, 0x01, 0xc0],
        );
        let vps: &[u8] = &[0x40, 0x01, 0x0c, 0x01];
        assert_eq!(hvcc[0], 1);
        assert_eq!(hvcc[1], 0x01);
        assert_eq!(&hvcc[2..6], &0x6000_0000u32.to_be_bytes());
        assert_eq!(hvcc[12], 120);
        assert_eq!(hvcc[21], 0x0f);
        assert_eq!(hvcc[22], 3);
        assert_eq!(hvcc[23], 0x80 | 32);
        assert_eq!(u16::from_be_bytes([hvcc[24], hvcc[25]]), 1);
        let vps_length = u16::from_be_bytes([hvcc[26], hvcc[27]]) as usize;
        assert_eq!(vps_length, vps.len());
        assert_eq!(&hvcc[28..28 + vps_length], vps);
        let sps_array = 28 + vps_length;
        assert_eq!(hvcc[sps_array], 0x80 | 33);
        let sps_length = u16::from_be_bytes([hvcc[sps_array + 3], hvcc[sps_array + 4]]) as usize;
        assert_eq!(sps_length, sps_nal.len());
        assert_eq!(
            &hvcc[sps_array + 5..sps_array + 5 + sps_length],
            sps_nal.as_slice()
        );
        let pps_array = sps_array + 5 + sps_length;
        assert_eq!(hvcc[pps_array], 0x80 | 34);
    }

    #[test]
    fn a_packet_sequence_gap_drops_the_partial_access_unit() {
        let mut assembler = Assembler::new();
        assert!(assembler
            .packet(10, false, &[vec![0x40, 0x01, 0x0c]], 90_000)
            .is_none());
        assert!(assembler
            .packet(13, true, &[vec![0x02, 0x01, 0xbb]], 91_000)
            .is_none());
        assert!(!assembler.synced());
        let unit = assembler
            .packet(14, true, &[vec![0x26, 0x01, 0xaf]], 92_000)
            .expect("a keyframe restores the assembler");
        assert!(unit.keyframe);
        assert_eq!(unit.nals.len(), 1);
        assert_eq!(unit.pts_us, 1_022_222);
    }

    #[test]
    fn the_marker_closes_one_access_unit_with_length_prefixed_nals() {
        let mut assembler = Assembler::new();
        let first = vec![0x26, 0x01, 0xaa];
        let second = vec![0x02, 0x01, 0xbb];
        assert!(assembler
            .packet(1, false, std::slice::from_ref(&first), 0)
            .is_none());
        let unit = assembler
            .packet(2, true, std::slice::from_ref(&second), 0)
            .expect("marker closes the unit");
        let bytes = avcc(&unit.nals);
        assert_eq!(bytes[..4], 3u32.to_be_bytes());
        assert_eq!(&bytes[4..7], first.as_slice());
        assert_eq!(bytes[7..11], 3u32.to_be_bytes());
        assert_eq!(&bytes[11..14], second.as_slice());
        assert!(unit.keyframe);
    }
}
