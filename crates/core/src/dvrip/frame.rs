use super::{HEADER_LEN, MAGIC, MAX_BODY};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub session: u32,
    pub sequence: u32,
    pub channel: u8,
    pub end: bool,
    pub id: u16,
    pub length: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    NotEnoughData,
    BadMagic(u8),
    TooLarge(u32),
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0] = MAGIC;
        out[4..8].copy_from_slice(&self.session.to_le_bytes());
        out[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        out[12] = self.channel;
        out[13] = u8::from(self.end);
        out[14..16].copy_from_slice(&self.id.to_le_bytes());
        out[16..20].copy_from_slice(&self.length.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> Result<Header, FrameError> {
        if buf.len() < HEADER_LEN {
            return Err(FrameError::NotEnoughData);
        }
        if buf[0] != MAGIC {
            return Err(FrameError::BadMagic(buf[0]));
        }
        let length = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        if length > MAX_BODY {
            return Err(FrameError::TooLarge(length));
        }
        Ok(Header {
            session: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            sequence: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            channel: buf[12],
            end: buf[13] != 0,
            id: u16::from_le_bytes([buf[14], buf[15]]),
            length,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_preserves_every_field() {
        let header = Header {
            session: 0xdead_beef,
            sequence: 42,
            channel: 7,
            end: true,
            id: 1006,
            length: 512,
        };
        let encoded = header.encode();
        assert_eq!(encoded[0], MAGIC);
        assert_eq!(Header::decode(&encoded).unwrap(), header);
    }

    #[test]
    fn short_buffer_is_incomplete_not_garbage() {
        assert_eq!(
            Header::decode(&[0xff, 0x00, 0x00]),
            Err(FrameError::NotEnoughData)
        );
    }

    #[test]
    fn wrong_magic_is_rejected() {
        let mut encoded = Header {
            session: 1,
            sequence: 1,
            channel: 0,
            end: true,
            id: 1000,
            length: 0,
        }
        .encode();
        encoded[0] = 0x00;
        assert_eq!(Header::decode(&encoded), Err(FrameError::BadMagic(0)));
    }

    #[test]
    fn oversized_length_is_rejected_before_allocation() {
        let mut encoded = Header {
            session: 1,
            sequence: 1,
            channel: 0,
            end: true,
            id: 1000,
            length: 0,
        }
        .encode();
        encoded[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Header::decode(&encoded),
            Err(FrameError::TooLarge(u32::MAX))
        );
    }
}
