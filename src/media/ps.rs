#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EsChunk {
    pub stream_id: u8,
    pub payload: Vec<u8>,
}

impl EsChunk {
    pub fn is_video(&self) -> bool {
        (0xe0..=0xef).contains(&self.stream_id)
    }

    pub fn is_audio(&self) -> bool {
        (0xc0..=0xdf).contains(&self.stream_id)
    }
}

#[derive(Default)]
pub struct PsDemuxer {
    buffer: Vec<u8>,
}

fn find_start_code(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn is_ps_stream_id(id: u8) -> bool {
    matches!(id, 0xb9 | 0xba..=0xbf | 0xc0..=0xef | 0xfd)
}

fn find_ps_start_code(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 4 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 && is_ps_stream_id(buf[i + 3]) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn keep_partial_start_code(buffer: &mut Vec<u8>) {
    let keep = if buffer.ends_with(&[0, 0]) {
        2
    } else if buffer.ends_with(&[0]) {
        1
    } else {
        0
    };
    let drain_to = buffer.len() - keep;
    buffer.drain(..drain_to);
}

impl PsDemuxer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    pub fn drain(&mut self) -> Vec<EsChunk> {
        let mut chunks = Vec::new();
        loop {
            let start = match find_start_code(&self.buffer, 0) {
                Some(index) => index,
                None => {
                    keep_partial_start_code(&mut self.buffer);
                    break;
                }
            };
            if start > 0 {
                self.buffer.drain(..start);
            }
            if self.buffer.len() < 4 {
                break;
            }
            let stream_id = self.buffer[3];
            match stream_id {
                0xba => {
                    if self.buffer.len() < 14 {
                        break;
                    }
                    let stuffing = (self.buffer[13] & 0x07) as usize;
                    let total = 14 + stuffing;
                    if self.buffer.len() < total {
                        break;
                    }
                    self.buffer.drain(..total);
                }
                0xb9 => {
                    self.buffer.drain(..4);
                }
                0xbb | 0xbc | 0xbe | 0xbf | 0xbd | 0xfd => {
                    if self.buffer.len() < 6 {
                        break;
                    }
                    let length = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                    let total = 6 + length;
                    if self.buffer.len() < total {
                        break;
                    }
                    self.buffer.drain(..total);
                }
                id if (0xc0..=0xef).contains(&id) => {
                    if self.buffer.len() < 9 {
                        break;
                    }
                    let declared = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                    let header_len = self.buffer[8] as usize;
                    let payload_start = 9 + header_len;
                    let total = if declared == 0 {
                        let next = find_ps_start_code(&self.buffer, payload_start);
                        match next {
                            Some(at) => at,
                            None => break,
                        }
                    } else {
                        let packet = 6 + declared;
                        if self.buffer.len() < packet {
                            break;
                        }
                        packet
                    };
                    if self.buffer.len() < payload_start || payload_start > total {
                        self.buffer.drain(..4);
                        continue;
                    }
                    let payload = self.buffer[payload_start..total].to_vec();
                    self.buffer.drain(..total);
                    if !payload.is_empty() {
                        chunks.push(EsChunk { stream_id, payload });
                    }
                }
                _ => {
                    self.buffer.drain(..4);
                }
            }
        }
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_header() -> Vec<u8> {
        let mut out = vec![0, 0, 1, 0xba];
        out.extend_from_slice(&[0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x00, 0x00, 0x03, 0xf8]);
        out
    }

    fn pes(stream_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, stream_id];
        let length = (3 + payload.len()) as u16;
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&[0x80, 0x00, 0x00]);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn splits_video_and_audio_elementary_streams() {
        let mut ps = pack_header();
        ps.extend(pes(0xe0, b"VIDEO"));
        ps.extend(pes(0xc0, b"AUDIO"));
        let mut demuxer = PsDemuxer::new();
        demuxer.push(&ps);
        let chunks = demuxer.drain();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].payload, b"VIDEO");
        assert!(chunks[0].is_video());
        assert_eq!(chunks[1].payload, b"AUDIO");
        assert!(chunks[1].is_audio());
    }

    #[test]
    fn reassembles_a_packet_split_across_pushes() {
        let mut ps = pack_header();
        ps.extend(pes(0xe0, b"0123456789"));
        let mut demuxer = PsDemuxer::new();
        let mut collected = Vec::new();
        for byte in &ps {
            demuxer.push(&[*byte]);
            collected.extend(demuxer.drain());
        }
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].payload, b"0123456789");
    }

    #[test]
    fn unbounded_video_pes_stops_at_the_next_start_code() {
        let mut ps = pack_header();
        let mut video = vec![0, 0, 1, 0xe0];
        video.extend_from_slice(&0u16.to_be_bytes());
        video.extend_from_slice(&[0x80, 0x00, 0x00]);
        video.extend_from_slice(&[0, 0, 1, 0x65, 0x11, 0x22]);
        ps.extend(video);
        ps.extend(pes(0xc0, b"AU"));
        let mut demuxer = PsDemuxer::new();
        demuxer.push(&ps);
        let chunks = demuxer.drain();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].payload, vec![0, 0, 1, 0x65, 0x11, 0x22]);
        assert_eq!(chunks[1].payload, b"AU");
    }

    #[test]
    fn a_start_code_split_across_pushes_is_not_lost() {
        let mut demuxer = PsDemuxer::new();
        demuxer.push(&[0x00, 0x00]);
        assert!(demuxer.drain().is_empty());
        let mut rest = pack_header();
        rest.extend(pes(0xe0, b"Z"));
        demuxer.push(&rest);
        let chunks = demuxer.drain();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].payload, b"Z");
    }

    #[test]
    fn pack_stuffing_bytes_are_consumed() {
        let mut pack = vec![0, 0, 1, 0xba];
        pack.extend_from_slice(&[0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x00, 0x00, 0x03, 0xf8]);
        pack.push(0x01);
        pack.push(0xff);
        pack.extend(pes(0xe0, b"X"));
        let mut demuxer = PsDemuxer::new();
        demuxer.push(&pack);
        let chunks = demuxer.drain();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].payload, b"X");
    }
}
