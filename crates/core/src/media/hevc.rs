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

pub fn is_picture_start(nal: &[u8]) -> bool {
    is_keyframe(nal) && nal.get(2).map(|byte| byte & 0x80 != 0).unwrap_or(false)
}

#[derive(Default)]
pub struct Assembler {
    nals: Vec<Vec<u8>>,
    keyframe: bool,
    timestamp: u32,
    unwrapped: Option<u64>,
    base: Option<u64>,
    in_sync: bool,
    boundary: bool,
}

impl Assembler {
    pub fn new() -> Self {
        Self {
            in_sync: true,
            boundary: true,
            ..Self::default()
        }
    }

    pub fn synced(&self) -> bool {
        self.in_sync
    }

    pub fn gap(&mut self) {
        self.nals.clear();
        self.keyframe = false;
        self.in_sync = false;
        self.boundary = true;
    }

    pub fn packet(&mut self, marker: bool, nals: &[Vec<u8>], timestamp: u32) -> Option<AccessUnit> {
        if !self.in_sync {
            if !self.boundary || !nals.iter().any(|nal| is_picture_start(nal)) {
                return None;
            }
            self.in_sync = true;
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
            self.boundary = false;
            None
        }
    }

    fn finish(&mut self) -> Option<AccessUnit> {
        self.boundary = true;
        if self.nals.is_empty() {
            return None;
        }
        let nals = std::mem::take(&mut self.nals);
        let keyframe = std::mem::take(&mut self.keyframe);
        let unwrapped = match self.unwrapped {
            None => (1u64 << 32) + u64::from(self.timestamp),
            Some(previous) => {
                let delta = self.timestamp.wrapping_sub(previous as u32) as i32;
                (previous as i64 + i64::from(delta)).max(0) as u64
            }
        };
        self.unwrapped = Some(unwrapped);
        let base = *self.base.get_or_insert(unwrapped);
        let pts_us = (unwrapped.saturating_sub(base) * 1_000_000) / 90_000;
        Some(AccessUnit {
            nals,
            keyframe,
            pts_us,
        })
    }
}

pub struct Reader {
    depacketizer: Depacketizer,
    assembler: Assembler,
    expected: Option<u16>,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

impl Reader {
    pub fn new() -> Self {
        Self {
            depacketizer: Depacketizer::new(),
            assembler: Assembler::new(),
            expected: None,
        }
    }

    pub fn synced(&self) -> bool {
        self.assembler.synced()
    }

    pub fn dropped(&self) -> u64 {
        self.depacketizer.dropped()
    }

    pub fn push(&mut self, packet: &RtpPacket<'_>) -> Option<AccessUnit> {
        if let Some(expected) = self.expected {
            if expected != packet.sequence {
                self.depacketizer.discard();
                self.assembler.gap();
            }
        }
        self.expected = Some(packet.sequence.wrapping_add(1));
        let nals = self.depacketizer.push(packet);
        self.assembler
            .packet(packet.marker, &nals, packet.timestamp)
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
        let sub_width: u32 = if chroma_format_idc == 1 || chroma_format_idc == 2 {
            2
        } else {
            1
        };
        let sub_height: u32 = if chroma_format_idc == 1 { 2 } else { 1 };
        width = width.saturating_sub(sub_width.saturating_mul(left.saturating_add(right)));
        height = height.saturating_sub(sub_height.saturating_mul(top.saturating_add(bottom)));
    }
    let bit_depth_luma = u8::try_from(bits.ue().ok_or("sps ended before the luma depth")?)
        .unwrap_or(8)
        .saturating_add(8);
    let bit_depth_chroma = u8::try_from(bits.ue().ok_or("sps ended before the chroma depth")?)
        .unwrap_or(8)
        .saturating_add(8);
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
    for byte in &bytes[2..end] {
        out.push('.');
        out.push_str(&format!("{byte:02X}"));
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

    pub fn discard(&mut self) {
        if self.fragment.take().is_some() {
            self.dropped += 1;
        }
    }

    pub fn partial(&self) -> bool {
        self.fragment.is_some()
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
    fn extra_constraint_bytes_are_dot_separated() {
        let profile = ProfileTierLevel {
            constraints: 0xB023_0000_0000,
            ..profile()
        };
        assert_eq!(codec_string(&profile), "hvc1.1.6.L120.B0.23");
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

    fn push_packet(
        reader: &mut Reader,
        marker: bool,
        sequence: u16,
        timestamp: u32,
        body: &[u8],
    ) -> Option<AccessUnit> {
        let mut raw = vec![0x80, if marker { 0x80 } else { 0 } | 98];
        raw.extend_from_slice(&sequence.to_be_bytes());
        raw.extend_from_slice(&timestamp.to_be_bytes());
        raw.extend_from_slice(&0xdead_beefu32.to_be_bytes());
        raw.extend_from_slice(body);
        let parsed = rtp::parse(&raw).expect("packet");
        reader.push(&parsed)
    }

    #[test]
    fn a_sequence_gap_drops_the_fragment_and_resyncs_on_a_picture_start() {
        let mut reader = Reader::new();
        assert!(push_packet(&mut reader, false, 1, 90_000, &[0x62, 0x01, 0x93, 0xaa]).is_none());
        assert!(push_packet(&mut reader, true, 3, 90_000, &[0x62, 0x01, 0x53, 0xbb]).is_none());
        assert!(!reader.synced());
        assert!(push_packet(&mut reader, true, 4, 90_000, &[0x26, 0x01, 0x00, 0xcc]).is_none());
        assert!(!reader.synced());
        let unit = push_packet(&mut reader, true, 5, 90_000, &[0x26, 0x01, 0x80, 0xdd])
            .expect("a picture start resynchronises");
        assert!(unit.keyframe);
        assert_eq!(unit.nals, vec![vec![0x26, 0x01, 0x80, 0xdd]]);
        assert!(reader.synced());
    }

    fn fragmented_idr(reader: &mut Reader, first: u16, timestamp: u32) -> Option<AccessUnit> {
        assert!(push_packet(
            reader,
            false,
            first,
            timestamp,
            &[0x62, 0x01, 0x93, 0x80, 0x11]
        )
        .is_none());
        assert!(push_packet(
            reader,
            false,
            first + 1,
            timestamp,
            &[0x62, 0x01, 0x13, 0x22]
        )
        .is_none());
        push_packet(
            reader,
            true,
            first + 2,
            timestamp,
            &[0x62, 0x01, 0x53, 0x33],
        )
    }

    #[test]
    fn a_fragmented_idr_after_a_gap_resyncs() {
        let mut reader = Reader::new();
        assert!(push_packet(
            &mut reader,
            false,
            1,
            3_000,
            &[0x62, 0x01, 0x93, 0x80, 0xaa]
        )
        .is_none());
        assert!(push_packet(&mut reader, true, 3, 3_000, &[0x62, 0x01, 0x53, 0xbb]).is_none());
        assert!(!reader.synced());
        let unit = fragmented_idr(&mut reader, 4, 9_000).expect("a fragmented idr resynchronises");
        assert!(unit.keyframe);
        assert_eq!(unit.nals, vec![vec![0x26, 0x01, 0x80, 0x11, 0x22, 0x33]]);
        assert!(reader.synced());
        assert!(fragmented_idr(&mut reader, 7, 18_000).is_some());
    }

    #[test]
    fn a_gap_landing_on_an_idr_fu_start_resyncs_on_the_next_idr() {
        let mut reader = Reader::new();
        assert!(push_packet(&mut reader, true, 1, 3_000, &[0x02, 0x01, 0x80, 0x01]).is_some());
        let unit =
            fragmented_idr(&mut reader, 5, 9_000).expect("the idr after the gap resynchronises");
        assert!(unit.keyframe);
        assert!(reader.synced());
    }

    #[test]
    fn a_lost_middle_fragment_does_not_publish_a_truncated_keyframe() {
        let mut reader = Reader::new();
        assert!(push_packet(
            &mut reader,
            false,
            1,
            9_000,
            &[0x62, 0x01, 0x93, 0x80, 0xaa]
        )
        .is_none());
        let unit = push_packet(&mut reader, true, 3, 9_000, &[0x62, 0x01, 0x53, 0xcc]);
        assert!(
            unit.is_none(),
            "published {:02x?}",
            unit.map(|unit| unit.nals)
        );
        assert!(!reader.synced());
    }

    #[test]
    fn timestamps_unwrap_a_forward_wrap_and_a_backward_step() {
        let mut assembler = Assembler::new();
        let nals = vec![vec![0x26u8, 0x01, 0x80]];
        let start = u32::MAX - 44_999;
        assert_eq!(assembler.packet(true, &nals, start).unwrap().pts_us, 0);
        assert_eq!(
            assembler.packet(true, &nals, 45_000).unwrap().pts_us,
            1_000_000
        );
        assert_eq!(
            assembler.packet(true, &nals, 36_000).unwrap().pts_us,
            900_000
        );
    }

    #[test]
    fn the_marker_closes_one_access_unit_with_length_prefixed_nals() {
        let mut assembler = Assembler::new();
        let first = vec![0x26, 0x01, 0xaa];
        let second = vec![0x02, 0x01, 0xbb];
        assert!(assembler
            .packet(false, std::slice::from_ref(&first), 0)
            .is_none());
        let unit = assembler
            .packet(true, std::slice::from_ref(&second), 0)
            .expect("marker closes the unit");
        let bytes = avcc(&unit.nals);
        assert_eq!(bytes[..4], 3u32.to_be_bytes());
        assert_eq!(&bytes[4..7], first.as_slice());
        assert_eq!(bytes[7..11], 3u32.to_be_bytes());
        assert_eq!(&bytes[11..14], second.as_slice());
        assert!(unit.keyframe);
    }

    struct Writer {
        bits: Vec<u8>,
    }

    impl Writer {
        fn new() -> Self {
            Self { bits: Vec::new() }
        }

        fn u(&mut self, value: u32, count: u32) {
            for index in (0..count).rev() {
                self.bits.push(((value >> index) & 1) as u8);
            }
        }

        fn ue(&mut self, value: u32) {
            let code = value + 1;
            let width = 32 - code.leading_zeros();
            self.u(0, width - 1);
            self.u(code, width);
        }

        fn bytes(&self) -> Vec<u8> {
            let mut out = vec![0u8; self.bits.len().div_ceil(8)];
            for (index, bit) in self.bits.iter().enumerate() {
                if *bit == 1 {
                    out[index / 8] |= 1 << (7 - index % 8);
                }
            }
            out
        }
    }

    #[test]
    fn the_camera_sps_parses_to_its_real_size_and_codec() {
        let sps = [
            0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03,
            0x00, 0x00, 0x03, 0x00, 0x3f, 0xa0, 0x01, 0x20, 0x20, 0x02, 0x88, 0x7f, 0x9c, 0xba,
            0xbc, 0x12, 0xe0, 0x80,
        ];
        let parsed = parse_sps(&sps).expect("camera sps");
        assert_eq!(parsed.width, 2304);
        assert_eq!(parsed.height, 2592);
        assert_eq!(parsed.chroma_format_idc, 1);
        assert_eq!(parsed.bit_depth_luma, 8);
        assert_eq!(codec_string(&parsed.profile), "hvc1.1.6.L63");
    }

    #[test]
    fn a_sps_with_sub_layers_is_parsed_past_them() {
        let mut writer = Writer::new();
        writer.u(0, 4);
        writer.u(2, 3);
        writer.u(1, 1);
        writer.u(0, 2);
        writer.u(0, 1);
        writer.u(1, 5);
        writer.u(0x6000_0000, 32);
        for _ in 0..6 {
            writer.u(0, 8);
        }
        writer.u(120, 8);
        writer.u(1, 1);
        writer.u(1, 1);
        writer.u(1, 1);
        writer.u(1, 1);
        for _ in 0..6 {
            writer.u(0, 2);
        }
        for _ in 0..2 {
            for _ in 0..11 {
                writer.u(0, 8);
            }
            writer.u(120, 8);
        }
        writer.ue(0);
        writer.ue(1);
        writer.ue(1920);
        writer.ue(1080);
        writer.u(0, 1);
        writer.ue(0);
        writer.ue(0);
        writer.u(1, 1);
        let mut nal = vec![0x42, 0x01];
        nal.extend_from_slice(&writer.bytes());
        let parsed = parse_sps(&nal).expect("synthetic sps");
        assert_eq!(parsed.width, 1920);
        assert_eq!(parsed.height, 1080);
        assert_eq!(parsed.profile.level_idc, 120);
        assert_eq!(parsed.chroma_format_idc, 1);
        assert_eq!(parsed.bit_depth_luma, 8);
    }

    #[test]
    fn only_the_first_slice_of_an_irap_counts_as_a_picture_start() {
        assert!(is_picture_start(&[0x26, 0x01, 0x80]));
        assert!(!is_picture_start(&[0x26, 0x01, 0x00]));
        assert!(!is_picture_start(&[0x02, 0x01, 0x80]));
    }
}
