use super::Codec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalUnit<'a> {
    pub data: &'a [u8],
}

impl<'a> NalUnit<'a> {
    pub fn h264_type(&self) -> Option<u8> {
        self.data.first().map(|b| b & 0x1f)
    }

    pub fn h265_type(&self) -> Option<u8> {
        self.data.first().map(|b| (b >> 1) & 0x3f)
    }

    pub fn is_vcl_h264(&self) -> bool {
        matches!(self.h264_type(), Some(1) | Some(5))
    }

    pub fn is_vcl_h265(&self) -> bool {
        (0..=31).contains(&self.h265_type().unwrap_or(255))
    }
}

fn start_code_len(buf: &[u8], at: usize) -> Option<usize> {
    if at + 3 <= buf.len() && buf[at] == 0 && buf[at + 1] == 0 && buf[at + 2] == 1 {
        return Some(3);
    }
    if at + 4 <= buf.len()
        && buf[at] == 0
        && buf[at + 1] == 0
        && buf[at + 2] == 0
        && buf[at + 3] == 1
    {
        return Some(4);
    }
    None
}

pub fn find_start_code(buf: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= buf.len() {
        if let Some(len) = start_code_len(buf, i) {
            return Some((i, len));
        }
        i += 1;
    }
    None
}

pub fn split_annexb(buf: &[u8]) -> Vec<NalUnit<'_>> {
    let mut units = Vec::new();
    let mut cursor = match find_start_code(buf, 0) {
        Some((at, len)) => at + len,
        None => return units,
    };
    loop {
        let next = find_start_code(buf, cursor);
        let end = match next {
            Some((at, _)) => at,
            None => buf.len(),
        };
        let slice = &buf[cursor..end];
        let trimmed = strip_trailing_zeros(slice);
        if !trimmed.is_empty() {
            units.push(NalUnit { data: trimmed });
        }
        match next {
            Some((at, len)) => cursor = at + len,
            None => break,
        }
    }
    units
}

fn strip_trailing_zeros(slice: &[u8]) -> &[u8] {
    let mut end = slice.len();
    while end > 0 && slice[end - 1] == 0 {
        end -= 1;
    }
    &slice[..end]
}

pub fn detect_codec(buf: &[u8]) -> Option<Codec> {
    let units = split_annexb(buf);
    if units.iter().any(|u| u.is_vcl_h264()) {
        return Some(Codec::H264);
    }
    if units
        .iter()
        .any(|u| u.is_vcl_h265() && u.h264_type() != Some(1) && u.h264_type() != Some(5))
    {
        return Some(Codec::H265);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_four_byte_and_three_byte_start_codes() {
        let stream = [
            &[0, 0, 0, 1, 0x67, 0x42][..],
            &[0, 0, 1, 0x68, 0xce][..],
            &[0, 0, 0, 1, 0x65, 0x88, 0x84][..],
        ]
        .concat();
        let units = split_annexb(&stream);
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].h264_type(), Some(7));
        assert_eq!(units[1].h264_type(), Some(8));
        assert_eq!(units[2].h264_type(), Some(5));
    }

    #[test]
    fn trailing_zero_bytes_are_not_part_of_the_nal() {
        let stream = [0, 0, 0, 1, 0x65, 0x88, 0x00, 0x00, 0, 0, 0, 1, 0x41, 0x9a];
        let units = split_annexb(&stream);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].data, &[0x65, 0x88]);
        assert_eq!(units[1].data, &[0x41, 0x9a]);
    }

    #[test]
    fn buffer_without_start_code_yields_nothing() {
        assert!(split_annexb(&[1, 2, 3, 4, 5]).is_empty());
    }

    #[test]
    fn truncated_start_code_at_the_end_is_not_a_nal() {
        let stream = [0, 0, 0, 1, 0x65, 0x01, 0x02, 0, 0, 1];
        let units = split_annexb(&stream);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].data, &[0x65, 0x01, 0x02]);
    }

    #[test]
    fn detects_h264_from_a_vcl_nal() {
        let stream = [0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x65, 0x88];
        assert_eq!(detect_codec(&stream), Some(Codec::H264));
    }

    #[test]
    fn detects_h265_from_a_vcl_nal() {
        let stream = [0, 0, 0, 1, 0x40, 0x01, 0x0c, 0, 0, 0, 1, 0x26, 0x01, 0xaf];
        assert_eq!(detect_codec(&stream), Some(Codec::H265));
    }

    #[test]
    fn codec_is_unknown_without_a_vcl_nal() {
        let stream = [0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68, 0xce];
        assert_eq!(detect_codec(&stream), None);
    }
}
