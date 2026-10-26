#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpsInfo {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub width: u32,
    pub height: u32,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
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

    Ok(SpsInfo {
        profile_idc,
        level_idc,
        width,
        height,
        sps: nal.to_vec(),
        pps: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
