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
}
