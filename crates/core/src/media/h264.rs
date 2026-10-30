use std::collections::VecDeque;

use crate::media::rtp::RtpPacket;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpsInfo {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub width: u32,
    pub height: u32,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
    pub may_reorder: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SpsError {
    Empty,
    NotSps,
    Truncated,
}

pub struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    pub fn bit(&mut self) -> Option<u32> {
        let byte = self.bit / 8;
        if byte >= self.data.len() {
            return None;
        }
        let shift = 7 - (self.bit % 8);
        self.bit += 1;
        Some(((self.data[byte] >> shift) & 1) as u32)
    }

    pub fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | self.bit()?;
        }
        Some(value)
    }

    pub fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0u32;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        if zeros == 0 {
            return Some(0);
        }
        let rest = self.bits(zeros)?;
        Some((1u32 << zeros) - 1 + rest)
    }

    pub fn se(&mut self) -> Option<i32> {
        let code = self.ue()? as i64;
        let value = if code % 2 == 0 {
            -(code / 2)
        } else {
            (code + 1) / 2
        };
        Some(value as i32)
    }
}

pub fn strip_emulation_prevention(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &byte in nal {
        if zeros == 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        if byte == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
        out.push(byte);
    }
    out
}

pub fn parse_sps(nal: &[u8]) -> Result<SpsInfo, SpsError> {
    if nal.is_empty() {
        return Err(SpsError::Empty);
    }
    if nal[0] & 0x1f != 7 {
        return Err(SpsError::NotSps);
    }
    let rbsp = strip_emulation_prevention(nal);
    if rbsp.len() < 4 {
        return Err(SpsError::Truncated);
    }
    let profile_idc = rbsp[1];
    let level_idc = rbsp[3];
    let mut reader = BitReader::new(&rbsp[4..]);

    reader.ue().ok_or(SpsError::Truncated)?;

    let high_profile = matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    );
    if high_profile {
        let chroma_format_idc = reader.ue().ok_or(SpsError::Truncated)?;
        if chroma_format_idc == 3 {
            reader.bit().ok_or(SpsError::Truncated)?;
        }
        reader.ue().ok_or(SpsError::Truncated)?;
        reader.ue().ok_or(SpsError::Truncated)?;
        reader.bit().ok_or(SpsError::Truncated)?;
        if reader.bit().ok_or(SpsError::Truncated)? == 1 {
            let count = if chroma_format_idc == 3 { 12 } else { 8 };
            for index in 0..count {
                if reader.bit().ok_or(SpsError::Truncated)? == 1 {
                    let size = if index < 6 { 16 } else { 64 };
                    let mut last = 8i32;
                    let mut next = 8i32;
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + reader.se().ok_or(SpsError::Truncated)? + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }

    reader.ue().ok_or(SpsError::Truncated)?;

    let pic_order_cnt_type = reader.ue().ok_or(SpsError::Truncated)?;
    if pic_order_cnt_type == 0 {
        reader.ue().ok_or(SpsError::Truncated)?;
    } else if pic_order_cnt_type == 1 {
        reader.bit().ok_or(SpsError::Truncated)?;
        reader.se().ok_or(SpsError::Truncated)?;
        reader.se().ok_or(SpsError::Truncated)?;
        let cycle = reader.ue().ok_or(SpsError::Truncated)?;
        for _ in 0..cycle {
            reader.se().ok_or(SpsError::Truncated)?;
        }
    }

    reader.ue().ok_or(SpsError::Truncated)?;
    reader.bit().ok_or(SpsError::Truncated)?;

    let width_mbs = reader.ue().ok_or(SpsError::Truncated)? + 1;
    let height_map_units = reader.ue().ok_or(SpsError::Truncated)? + 1;
    let frame_mbs_only = reader.bit().ok_or(SpsError::Truncated)?;
    if frame_mbs_only == 0 {
        reader.bit().ok_or(SpsError::Truncated)?;
    }
    reader.bit().ok_or(SpsError::Truncated)?;

    let mut crop_left = 0;
    let mut crop_right = 0;
    let mut crop_top = 0;
    let mut crop_bottom = 0;
    if reader.bit().ok_or(SpsError::Truncated)? == 1 {
        crop_left = reader.ue().ok_or(SpsError::Truncated)?;
        crop_right = reader.ue().ok_or(SpsError::Truncated)?;
        crop_top = reader.ue().ok_or(SpsError::Truncated)?;
        crop_bottom = reader.ue().ok_or(SpsError::Truncated)?;
    }

    let width = width_mbs * 16 - 2 * (crop_left + crop_right);
    let height = (2 - frame_mbs_only) * height_map_units * 16 - 2 * (crop_top + crop_bottom);
    let declared = if reader.bit() == Some(1) {
        vui_reorder(&mut reader)
    } else {
        None
    };
    let may_reorder = match declared {
        Some(frames) => frames > 0,
        None => profile_idc != 66 && pic_order_cnt_type != 2,
    };

    Ok(SpsInfo {
        profile_idc,
        level_idc,
        width,
        height,
        sps: nal.to_vec(),
        pps: Vec::new(),
        may_reorder,
    })
}

fn skip_hrd(reader: &mut BitReader<'_>) -> Option<()> {
    let count = reader.ue()? + 1;
    reader.bits(8)?;
    for _ in 0..count {
        reader.ue()?;
        reader.ue()?;
        reader.bit()?;
    }
    reader.bits(20)?;
    Some(())
}

fn vui_reorder(reader: &mut BitReader<'_>) -> Option<u32> {
    if reader.bit()? == 1 && reader.bits(8)? == 255 {
        reader.bits(32)?;
    }
    if reader.bit()? == 1 {
        reader.bit()?;
    }
    if reader.bit()? == 1 {
        reader.bits(4)?;
        if reader.bit()? == 1 {
            reader.bits(24)?;
        }
    }
    if reader.bit()? == 1 {
        reader.ue()?;
        reader.ue()?;
    }
    if reader.bit()? == 1 {
        reader.bits(32)?;
        reader.bits(32)?;
        reader.bit()?;
    }
    let nal_hrd = reader.bit()? == 1;
    if nal_hrd {
        skip_hrd(reader)?;
    }
    let vcl_hrd = reader.bit()? == 1;
    if vcl_hrd {
        skip_hrd(reader)?;
    }
    if nal_hrd || vcl_hrd {
        reader.bit()?;
    }
    reader.bit()?;
    if reader.bit()? == 0 {
        return None;
    }
    reader.bit()?;
    for _ in 0..4 {
        reader.ue()?;
    }
    reader.ue()
}

pub const STAP_A: u8 = 24;
pub const FU_A: u8 = 28;

pub fn nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|byte| byte & 0x1f)
}

pub fn is_keyframe(nal: &[u8]) -> bool {
    nal_type(nal) == Some(5)
}

pub fn is_vcl(nal: &[u8]) -> bool {
    matches!(nal_type(nal), Some(1..=5))
}

pub fn is_picture_start(nal: &[u8]) -> bool {
    is_vcl(nal) && nal.get(1).map(|byte| byte & 0x80 != 0).unwrap_or(false)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit {
    pub nals: Vec<Vec<u8>>,
    pub keyframe: bool,
    pub pts_us: u64,
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

    pub fn partial(&self) -> bool {
        self.fragment.is_some()
    }

    pub fn discard(&mut self) {
        if self.fragment.take().is_some() {
            self.dropped += 1;
        }
    }

    pub fn push(&mut self, packet: &RtpPacket<'_>) -> Vec<Vec<u8>> {
        let payload = packet.payload;
        match nal_type(payload) {
            Some(STAP_A) => self.aggregation(&payload[1..]),
            Some(FU_A) => self.fragmentation(payload),
            Some(_) => {
                self.discard();
                vec![payload.to_vec()]
            }
            None => {
                self.discard();
                Vec::new()
            }
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
        let indicator = payload[0];
        let header = payload[1];
        let start = header & 0x80 != 0;
        let end = header & 0x40 != 0;
        let kind = header & 0x1f;
        if start {
            self.discard();
            self.fragment = Some(Fragment {
                nal: vec![(indicator & 0xe0) | kind],
            });
            if let Some(fragment) = self.fragment.as_mut() {
                fragment.nal.extend_from_slice(&payload[2..]);
            }
            return if end { self.take() } else { Vec::new() };
        }
        match self.fragment.as_mut() {
            Some(fragment) => {
                fragment.nal.extend_from_slice(&payload[2..]);
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

pub const MAX_ACCESS_UNIT_BYTES: usize = 8 * 1024 * 1024;

pub struct Reader {
    depacketizer: Depacketizer,
    nals: Vec<Vec<u8>>,
    bytes: usize,
    keyframe: bool,
    timestamp: u32,
    unwrapped: Option<u64>,
    base: Option<u64>,
    expected: Option<u16>,
    in_sync: bool,
    boundary: bool,
    ready: VecDeque<AccessUnit>,
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
            nals: Vec::new(),
            bytes: 0,
            keyframe: false,
            timestamp: 0,
            unwrapped: None,
            base: None,
            expected: None,
            in_sync: true,
            boundary: true,
            ready: VecDeque::new(),
        }
    }

    pub fn dropped(&self) -> u64 {
        self.depacketizer.dropped()
    }

    pub fn push(&mut self, packet: &RtpPacket<'_>) -> Option<AccessUnit> {
        if let Some(expected) = self.expected {
            if expected != packet.sequence {
                self.depacketizer.discard();
                self.gap();
            }
        }
        self.expected = Some(packet.sequence.wrapping_add(1));
        let nals = self.depacketizer.push(packet);
        if !self.in_sync {
            if !self.boundary || !nals.iter().any(|nal| is_picture_start(nal)) {
                return self.ready.pop_front();
            }
            self.in_sync = true;
        }
        if !self.nals.is_empty() && packet.timestamp != self.timestamp {
            self.close();
        }
        if self.nals.is_empty() {
            self.timestamp = packet.timestamp;
        }
        for nal in nals {
            if is_keyframe(&nal) {
                self.keyframe = true;
            }
            self.bytes += nal.len();
            self.nals.push(nal);
        }
        if self.bytes > MAX_ACCESS_UNIT_BYTES {
            self.gap();
        } else if packet.marker {
            self.close();
            self.boundary = true;
        } else {
            self.boundary = false;
        }
        self.ready.pop_front()
    }

    fn gap(&mut self) {
        self.nals.clear();
        self.bytes = 0;
        self.keyframe = false;
        self.in_sync = false;
        self.boundary = true;
    }

    fn close(&mut self) {
        if self.nals.is_empty() {
            return;
        }
        let nals = std::mem::take(&mut self.nals);
        let keyframe = std::mem::take(&mut self.keyframe);
        self.bytes = 0;
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
        self.ready.push_back(AccessUnit {
            nals,
            keyframe,
            pts_us,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::rtp;

    fn rtp_packet(sequence: u16, marker: bool, body: &[u8]) -> Vec<u8> {
        let mut raw = vec![0x80, if marker { 0x80 } else { 0 } | 96];
        raw.extend_from_slice(&sequence.to_be_bytes());
        raw.extend_from_slice(&0u32.to_be_bytes());
        raw.extend_from_slice(&0x1122_3344u32.to_be_bytes());
        raw.extend_from_slice(body);
        raw
    }

    fn feed(depacketizer: &mut Depacketizer, marker: bool, body: &[u8]) -> Vec<Vec<u8>> {
        let raw = rtp_packet(1, marker, body);
        let parsed = rtp::parse(&raw).expect("packet");
        depacketizer.push(&parsed)
    }

    #[test]
    fn fu_a_fragments_reassemble_into_the_original_nal() {
        let mut depacketizer = Depacketizer::new();
        let mut first = vec![0x7c, 0x85, 0x88, 0x84];
        assert!(feed(&mut depacketizer, false, &first).is_empty());
        first.clear();
        let last = vec![0x7c, 0x45, 0x00, 0x21, 0xff];
        let nals = feed(&mut depacketizer, true, &last);
        assert_eq!(nals, vec![vec![0x65, 0x88, 0x84, 0x00, 0x21, 0xff]]);
        assert_eq!(depacketizer.dropped(), 0);
    }

    #[test]
    fn stap_a_returns_every_aggregated_nal() {
        let mut depacketizer = Depacketizer::new();
        let sps = [0x67, 0x42, 0x00, 0x1e];
        let pps = [0x68, 0xce, 0x3c, 0x80];
        let mut body = vec![STAP_A];
        body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps);
        body.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        body.extend_from_slice(&pps);
        let nals = feed(&mut depacketizer, false, &body);
        assert_eq!(nals, vec![sps.to_vec(), pps.to_vec()]);
        assert_eq!(nal_type(&nals[0]), Some(7));
        assert_eq!(nal_type(&nals[1]), Some(8));
    }

    #[test]
    fn a_single_nal_packet_is_passed_through() {
        let mut depacketizer = Depacketizer::new();
        let nal = vec![0x41, 0x9a, 0x22];
        assert_eq!(feed(&mut depacketizer, false, &nal), vec![nal]);
    }

    #[test]
    fn a_fragment_that_never_started_is_dropped() {
        let mut depacketizer = Depacketizer::new();
        assert!(feed(&mut depacketizer, false, &[0x7c, 0x05, 0x11]).is_empty());
        assert_eq!(depacketizer.dropped(), 1);
    }

    fn reader_packets() -> Vec<(bool, Vec<u8>)> {
        let sps = [0x67, 0x42, 0x00, 0x1e];
        let pps = [0x68, 0xce, 0x3c, 0x80];
        let mut sets = vec![STAP_A];
        sets.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        sets.extend_from_slice(&sps);
        sets.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        sets.extend_from_slice(&pps);
        vec![
            (false, sets),
            (false, vec![0x7c, 0x85, 0x88, 0x84]),
            (true, vec![0x7c, 0x45, 0x00, 0x21]),
            (true, vec![0x41, 0x9a, 0x22]),
        ]
    }

    #[test]
    fn the_reader_emits_one_access_unit_per_marker() {
        let mut reader = Reader::new();
        let mut units = Vec::new();
        for (index, (marker, body)) in reader_packets().into_iter().enumerate() {
            let raw = rtp_packet(index as u16 + 1, marker, &body);
            let parsed = rtp::parse(&raw).expect("packet");
            if let Some(unit) = reader.push(&parsed) {
                units.push(unit);
            }
        }
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].nals.len(), 3);
        assert!(units[0].keyframe);
        assert_eq!(units[0].nals[2], vec![0x65, 0x88, 0x84, 0x00, 0x21]);
        assert_eq!(units[1].nals.len(), 1);
        assert!(!units[1].keyframe);
    }

    #[test]
    fn a_gap_drops_deltas_until_the_next_picture_start() {
        let mut reader = Reader::new();
        let raw = rtp_packet(1, true, &[0x65, 0x88, 0x84]);
        assert!(reader.push(&rtp::parse(&raw).expect("packet")).is_some());
        let raw = rtp_packet(9, true, &[0x41, 0x1a, 0x22]);
        assert!(reader.push(&rtp::parse(&raw).expect("packet")).is_none());
        let raw = rtp_packet(10, true, &[0x41, 0x9a, 0x22]);
        assert!(reader.push(&rtp::parse(&raw).expect("packet")).is_some());
    }

    struct BitWriter {
        bits: Vec<u8>,
    }

    impl BitWriter {
        fn new() -> Self {
            Self { bits: Vec::new() }
        }
        fn bit(&mut self, value: u32) {
            self.bits.push((value & 1) as u8);
        }
        fn ue(&mut self, value: u32) {
            let v = value + 1;
            let mut len = 0;
            while (v >> len) > 1 {
                len += 1;
            }
            for _ in 0..len {
                self.bit(0);
            }
            for index in (0..=len).rev() {
                self.bit((v >> index) & 1);
            }
        }
        fn finish(&self) -> Vec<u8> {
            let mut out = vec![0u8; self.bits.len().div_ceil(8)];
            for (index, bit) in self.bits.iter().enumerate() {
                if *bit == 1 {
                    out[index / 8] |= 1 << (7 - (index % 8));
                }
            }
            out
        }
    }

    fn baseline_sps(width: u32, height: u32) -> Vec<u8> {
        let mut writer = BitWriter::new();
        writer.ue(0);
        writer.ue(0);
        writer.ue(0);
        writer.ue(1);
        writer.ue(1);
        writer.bit(0);
        writer.ue(width / 16 - 1);
        writer.ue(height / 16 - 1);
        writer.bit(1);
        writer.bit(1);
        writer.bit(0);
        let body = writer.finish();
        let mut nal = vec![0x67, 66, 0x00, 30];
        nal.extend(body);
        nal
    }

    #[test]
    fn exp_golomb_decodes_seeded_values() {
        let mut writer = BitWriter::new();
        for value in [0u32, 1, 2, 3, 7, 8, 100] {
            writer.ue(value);
        }
        let data = writer.finish();
        let mut reader = BitReader::new(&data);
        for value in [0u32, 1, 2, 3, 7, 8, 100] {
            assert_eq!(reader.ue(), Some(value));
        }
    }

    #[test]
    fn signed_exp_golomb_alternates_around_zero() {
        let mut writer = BitWriter::new();
        for value in [0i32, 1, -1, 2, -2] {
            let code = if value <= 0 {
                (-value as u32) * 2
            } else {
                value as u32 * 2 - 1
            };
            writer.ue(code);
        }
        let data = writer.finish();
        let mut reader = BitReader::new(&data);
        for value in [0i32, 1, -1, 2, -2] {
            assert_eq!(reader.se(), Some(value));
        }
    }

    #[test]
    fn parses_dimensions_from_a_baseline_sps() {
        let sps = baseline_sps(1280, 720);
        let info = parse_sps(&sps).unwrap();
        assert_eq!(info.profile_idc, 66);
        assert_eq!(info.level_idc, 30);
        assert_eq!(info.width, 1280);
        assert_eq!(info.height, 720);
    }

    #[test]
    fn rejects_a_non_sps_nal() {
        assert_eq!(parse_sps(&[0x65, 0x88]), Err(SpsError::NotSps));
    }

    #[test]
    fn strips_emulation_prevention_bytes() {
        assert_eq!(
            strip_emulation_prevention(&[0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x00]),
            vec![0x00, 0x00, 0x01, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn truncated_sps_is_reported_not_guessed() {
        assert_eq!(parse_sps(&[0x67, 66, 0x00]), Err(SpsError::Truncated));
    }
}
