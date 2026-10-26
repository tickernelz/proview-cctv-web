#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpPacket<'a> {
    pub marker: bool,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: &'a [u8],
}

pub fn parse(data: &[u8]) -> Option<RtpPacket<'_>> {
    if data.len() < 12 || data[0] >> 6 != 2 {
        return None;
    }
    let padding = data[0] & 0x20 != 0;
    let extension = data[0] & 0x10 != 0;
    let csrc_count = (data[0] & 0x0f) as usize;
    let marker = data[1] & 0x80 != 0;
    let payload_type = data[1] & 0x7f;
    let sequence = u16::from_be_bytes([data[2], data[3]]);
    let timestamp = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let ssrc = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let mut offset = 12 + csrc_count * 4;
    if data.len() < offset {
        return None;
    }
    if extension {
        if data.len() < offset + 4 {
            return None;
        }
        let words = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset += 4 + words * 4;
        if data.len() < offset {
            return None;
        }
    }
    let mut end = data.len();
    if padding {
        let pad = *data.last()? as usize;
        if pad == 0 || pad > end - offset {
            return None;
        }
        end -= pad;
    }
    Some(RtpPacket {
        marker,
        payload_type,
        sequence,
        timestamp,
        ssrc,
        payload: &data[offset..end],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(marker: bool, payload_type: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0x80, if marker { 0x80 } else { 0 } | payload_type];
        out.extend_from_slice(&7u16.to_be_bytes());
        out.extend_from_slice(&90_000u32.to_be_bytes());
        out.extend_from_slice(&0xdead_beefu32.to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn header_fields_are_read_from_the_fixed_prefix() {
        let raw = packet(true, 98, &[1, 2, 3]);
        let parsed = parse(&raw).expect("packet");
        assert!(parsed.marker);
        assert_eq!(parsed.payload_type, 98);
        assert_eq!(parsed.sequence, 7);
        assert_eq!(parsed.timestamp, 90_000);
        assert_eq!(parsed.ssrc, 0xdead_beef);
        assert_eq!(parsed.payload, &[1, 2, 3]);
    }

    #[test]
    fn csrc_entries_and_extension_header_are_skipped() {
        let mut raw = vec![0x92, 96];
        raw.extend_from_slice(&1u16.to_be_bytes());
        raw.extend_from_slice(&0u32.to_be_bytes());
        raw.extend_from_slice(&1u32.to_be_bytes());
        raw.extend_from_slice(&0xaaaa_bbbbu32.to_be_bytes());
        raw.extend_from_slice(&0xcafe_babeu32.to_be_bytes());
        raw.extend_from_slice(&[0xb1, 0x2b]);
        raw.extend_from_slice(&1u16.to_be_bytes());
        raw.extend_from_slice(&0x1111_2222u32.to_be_bytes());
        raw.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        let parsed = parse(&raw).expect("packet");
        assert_eq!(parsed.payload, &[0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn padding_is_removed_from_the_payload() {
        let mut raw = packet(false, 98, &[9, 9, 0, 0, 2]);
        raw[0] |= 0x20;
        let parsed = parse(&raw).expect("packet");
        assert_eq!(parsed.payload, &[9, 9, 0]);
    }

    #[test]
    fn truncated_and_wrong_version_packets_are_rejected() {
        assert!(parse(&[0x80, 98, 0, 1]).is_none());
        let mut raw = packet(false, 98, &[1]);
        raw[0] = 0x40;
        assert!(parse(&raw).is_none());
        let mut raw = packet(false, 98, &[1]);
        raw[1] |= 0x7f;
        raw[0] |= 0x2f;
        assert!(parse(&raw).is_none());
    }
}
