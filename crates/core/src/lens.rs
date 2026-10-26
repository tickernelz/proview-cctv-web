use serde::{Deserialize, Serialize};

use crate::media::h264::{self, BitReader};

const SIXTEEN_BY_NINE: f32 = 9.0 / 16.0;
const RATIO_TOLERANCE: f32 = 0.02;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensCapabilities {
    pub multi_lens_two_sensor: bool,
    pub multi_lens_three_sensor: bool,
    pub support_scale_two_lens: bool,
    pub support_scale_three_lens: bool,
    pub support_gun_ball_two_sensor_ptz_locate: bool,
}

impl LensCapabilities {
    pub fn from_flag_list(list: &str) -> Self {
        let mut caps = Self::default();
        for raw in list.split(|unit: char| unit == ',' || unit == ';' || unit.is_whitespace()) {
            let name = raw.trim();
            if name.eq_ignore_ascii_case("MultiLensTwoSensor") {
                caps.multi_lens_two_sensor = true;
            } else if name.eq_ignore_ascii_case("MultiLensThreeSensor") {
                caps.multi_lens_three_sensor = true;
            } else if name.eq_ignore_ascii_case("SupportScaleTwoLens") {
                caps.support_scale_two_lens = true;
            } else if name.eq_ignore_ascii_case("SupportScaleThreeLens") {
                caps.support_scale_three_lens = true;
            } else if name.eq_ignore_ascii_case("SupportGunBallTwoSensorPtzLocate") {
                caps.support_gun_ball_two_sensor_ptz_locate = true;
            }
        }
        caps
    }

    pub fn declared_lenses(&self) -> Option<u32> {
        if self.multi_lens_three_sensor {
            Some(3)
        } else if self.multi_lens_two_sensor {
            Some(2)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensCountSource {
    Capability,
    Geometry,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensStack {
    Vertical,
    Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameSizeSource {
    SdpSps,
    ClientHint,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    pub fn measured(self) -> bool {
        self.width > 0 && self.height > 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LensLayout {
    pub count: u32,
    pub source: LensCountSource,
    pub stack: LensStack,
    pub geometry_count: Option<u32>,
}

impl LensLayout {
    pub fn crops_verified(&self) -> bool {
        self.count >= 2 && self.geometry_count == Some(self.count)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LensCrop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub fn geometry_lenses(frame: FrameSize) -> Option<u32> {
    if !frame.measured() {
        return None;
    }
    let ratio = frame.height as f32 / frame.width as f32;
    let stacked = [3u32, 2u32]
        .into_iter()
        .find(|count| (ratio - *count as f32 * SIXTEEN_BY_NINE).abs() <= RATIO_TOLERANCE);
    Some(stacked.unwrap_or(1))
}

pub fn detect_layout(frame: FrameSize, capabilities: LensCapabilities) -> LensLayout {
    let geometry_count = geometry_lenses(frame);
    let declared = capabilities.declared_lenses();
    let count = declared.or(geometry_count).unwrap_or(0);
    let source = if declared.is_some() {
        LensCountSource::Capability
    } else if geometry_count.is_some() {
        LensCountSource::Geometry
    } else {
        LensCountSource::Unknown
    };
    LensLayout {
        count,
        source,
        stack: if count >= 2 {
            LensStack::Vertical
        } else {
            LensStack::Single
        },
        geometry_count,
    }
}

pub fn stacked_crops(layout: &LensLayout) -> Vec<LensCrop> {
    if layout.count < 2 {
        return vec![LensCrop {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }];
    }
    let step = 1.0 / layout.count as f32;
    (0..layout.count)
        .map(|index| LensCrop {
            x: 0.0,
            y: index as f32 * step,
            w: 1.0,
            h: step,
        })
        .collect()
}

pub fn crop_pixels(crop: LensCrop, frame: FrameSize) -> CropRect {
    CropRect {
        x: (crop.x * frame.width as f32).round() as u32,
        y: (crop.y * frame.height as f32).round() as u32,
        w: (crop.w * frame.width as f32).round() as u32,
        h: (crop.h * frame.height as f32).round() as u32,
    }
}

pub fn lens_label(index: u32, count: u32) -> &'static str {
    match (count, index) {
        (0 | 1, _) => "full",
        (2, 0) => "top",
        (2, _) => "bottom",
        (3, 0) => "top",
        (3, 1) => "middle",
        _ => "bottom",
    }
}

pub fn frame_size_from_sps(codec: &str, sps: &[u8]) -> Option<FrameSize> {
    if sps.is_empty() {
        return None;
    }
    let name = codec.trim();
    if name.eq_ignore_ascii_case("H264")
        || name.eq_ignore_ascii_case("H.264")
        || name.eq_ignore_ascii_case("AVC")
    {
        let info = h264::parse_sps(sps).ok()?;
        return Some(FrameSize {
            width: info.width,
            height: info.height,
        })
        .filter(|size| size.measured());
    }
    if name.eq_ignore_ascii_case("H265")
        || name.eq_ignore_ascii_case("H.265")
        || name.eq_ignore_ascii_case("HEVC")
    {
        return hevc_sps_size(sps).filter(|size| size.measured());
    }
    None
}

fn skip_bits(reader: &mut BitReader<'_>, count: u32) -> Option<()> {
    let mut remaining = count;
    while remaining > 0 {
        let step = remaining.min(32);
        reader.bits(step)?;
        remaining -= step;
    }
    Some(())
}

fn chroma_units(chroma_format_idc: u32) -> Option<(u32, u32)> {
    match chroma_format_idc {
        0 | 3 => Some((1, 1)),
        1 => Some((2, 2)),
        2 => Some((2, 1)),
        _ => None,
    }
}

fn skip_profile_tier_level(reader: &mut BitReader<'_>, max_sub_layers_minus1: u32) -> Option<()> {
    skip_bits(reader, 2 + 1 + 5)?;
    skip_bits(reader, 32)?;
    skip_bits(reader, 48)?;
    skip_bits(reader, 8)?;
    let mut profile_present = [false; 8];
    let mut level_present = [false; 8];
    for slot in 0..max_sub_layers_minus1 as usize {
        profile_present[slot] = reader.bit()? == 1;
        level_present[slot] = reader.bit()? == 1;
    }
    if max_sub_layers_minus1 > 0 {
        skip_bits(reader, 2 * (8 - max_sub_layers_minus1))?;
    }
    for slot in 0..max_sub_layers_minus1 as usize {
        if profile_present[slot] {
            skip_bits(reader, 88)?;
        }
        if level_present[slot] {
            skip_bits(reader, 8)?;
        }
    }
    Some(())
}

fn hevc_sps_size(sps: &[u8]) -> Option<FrameSize> {
    if sps.len() < 2 || (sps[0] >> 1) & 0x3f != 33 {
        return None;
    }
    let rbsp = h264::strip_emulation_prevention(&sps[2..]);
    let mut reader = BitReader::new(&rbsp);
    skip_bits(&mut reader, 4)?;
    let max_sub_layers_minus1 = reader.bits(3)?;
    reader.bit()?;
    skip_profile_tier_level(&mut reader, max_sub_layers_minus1)?;
    reader.ue()?;
    let chroma_format_idc = reader.ue()?;
    if chroma_format_idc == 3 {
        reader.bit()?;
    }
    let mut width = reader.ue()?;
    let mut height = reader.ue()?;
    if reader.bit()? == 1 {
        let left = reader.ue()?;
        let right = reader.ue()?;
        let top = reader.ue()?;
        let bottom = reader.ue()?;
        let (unit_x, unit_y) = chroma_units(chroma_format_idc)?;
        width = width.saturating_sub(unit_x * (left + right));
        height = height.saturating_sub(unit_y * (top + bottom));
    }
    Some(FrameSize { width, height })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::nal::split_annexb;

    const DUAL_LENS_CLIP: &[u8] =
        include_bytes!("../../../fixtures/dual-lens/dual-lens-stacked-640x720.h264");
    const HEVC_SPS: &[u8] = include_bytes!("../../../fixtures/dual-lens/hevc-1920x1080.sps");

    fn clip_sps(codec_type: u8) -> Vec<u8> {
        split_annexb(DUAL_LENS_CLIP)
            .into_iter()
            .find(|unit| unit.h264_type() == Some(codec_type))
            .map(|unit| unit.data.to_vec())
            .expect("fixture clip carries the parameter set")
    }

    #[test]
    fn the_measured_main_stream_splits_into_two_vertical_lenses() {
        let frame = FrameSize {
            width: 2304,
            height: 2592,
        };
        let layout = detect_layout(frame, LensCapabilities::default());
        assert_eq!(layout.count, 2);
        assert_eq!(layout.source, LensCountSource::Geometry);
        assert_eq!(layout.stack, LensStack::Vertical);
        assert!(layout.crops_verified());

        let crops = stacked_crops(&layout);
        assert_eq!(
            crops[0],
            LensCrop {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 0.5
            }
        );
        assert_eq!(
            crops[1],
            LensCrop {
                x: 0.0,
                y: 0.5,
                w: 1.0,
                h: 0.5
            }
        );
        assert_eq!(
            crop_pixels(crops[0], frame),
            CropRect {
                x: 0,
                y: 0,
                w: 2304,
                h: 1296
            }
        );
        assert_eq!(
            crop_pixels(crops[1], frame),
            CropRect {
                x: 0,
                y: 1296,
                w: 2304,
                h: 1296
            }
        );
    }

    #[test]
    fn the_measured_sub_stream_splits_at_the_same_ratio() {
        let frame = FrameSize {
            width: 640,
            height: 720,
        };
        let layout = detect_layout(frame, LensCapabilities::default());
        assert_eq!(layout.count, 2);
        let crops = stacked_crops(&layout);
        assert_eq!(
            crop_pixels(crops[0], frame),
            CropRect {
                x: 0,
                y: 0,
                w: 640,
                h: 360
            }
        );
        assert_eq!(
            crop_pixels(crops[1], frame),
            CropRect {
                x: 0,
                y: 360,
                w: 640,
                h: 360
            }
        );
    }

    #[test]
    fn a_declared_two_sensor_lens_count_wins_over_geometry() {
        let frame = FrameSize {
            width: 1920,
            height: 1080,
        };
        let caps = LensCapabilities {
            multi_lens_two_sensor: true,
            ..LensCapabilities::default()
        };
        let layout = detect_layout(frame, caps);
        assert_eq!(layout.count, 2);
        assert_eq!(layout.source, LensCountSource::Capability);
        assert_eq!(layout.geometry_count, Some(1));
        assert!(!layout.crops_verified());
    }

    #[test]
    fn a_single_sixteen_by_nine_frame_is_one_lens() {
        let frame = FrameSize {
            width: 1920,
            height: 1080,
        };
        let layout = detect_layout(frame, LensCapabilities::default());
        assert_eq!(layout.count, 1);
        assert_eq!(layout.source, LensCountSource::Geometry);
        assert_eq!(layout.stack, LensStack::Single);
        assert!(!layout.crops_verified());
        let crops = stacked_crops(&layout);
        assert_eq!(crops.len(), 1);
        assert_eq!(
            crops[0],
            LensCrop {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0
            }
        );
        assert_eq!(lens_label(0, 1), "full");
    }

    #[test]
    fn an_unmeasured_frame_reports_no_lens_count() {
        let layout = detect_layout(
            FrameSize {
                width: 0,
                height: 0,
            },
            LensCapabilities::default(),
        );
        assert_eq!(layout.count, 0);
        assert_eq!(layout.source, LensCountSource::Unknown);
        assert_eq!(layout.geometry_count, None);
        assert!(!layout.crops_verified());
    }

    #[test]
    fn three_declared_sensors_split_into_thirds() {
        let frame = FrameSize {
            width: 2304,
            height: 3888,
        };
        let caps = LensCapabilities {
            multi_lens_three_sensor: true,
            ..LensCapabilities::default()
        };
        let layout = detect_layout(frame, caps);
        assert_eq!(layout.count, 3);
        assert_eq!(layout.source, LensCountSource::Capability);
        assert!(layout.crops_verified());
        let crops = stacked_crops(&layout);
        assert_eq!(crops.len(), 3);
        assert!((crops[1].y - 1.0 / 3.0).abs() < 1e-6);
        assert!((crops[2].y - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(lens_label(0, 3), "top");
        assert_eq!(lens_label(1, 3), "middle");
        assert_eq!(lens_label(2, 3), "bottom");
    }

    #[test]
    fn the_capability_flag_list_matches_the_apk_field_names() {
        let caps = LensCapabilities::from_flag_list(
            "multilenstwosensor, SupportGunBallTwoSensorPtzLocate;SupportScaleTwoLens",
        );
        assert_eq!(caps.declared_lenses(), Some(2));
        assert!(caps.support_gun_ball_two_sensor_ptz_locate);
        assert!(caps.support_scale_two_lens);
        assert!(!caps.multi_lens_three_sensor);
        assert_eq!(LensCapabilities::from_flag_list("").declared_lenses(), None);
    }

    #[test]
    fn the_dual_lens_fixture_is_a_two_lens_frame() {
        let sps = clip_sps(7);
        let frame = frame_size_from_sps("H264", &sps).expect("fixture SPS parses");
        assert_eq!(
            frame,
            FrameSize {
                width: 640,
                height: 720
            }
        );
        let layout = detect_layout(frame, LensCapabilities::default());
        assert_eq!(layout.count, 2);
        assert!(layout.crops_verified());
    }

    #[test]
    fn an_hevc_sps_reports_its_frame_size() {
        let frame = frame_size_from_sps("H265", HEVC_SPS).expect("hevc SPS parses");
        assert_eq!(
            frame,
            FrameSize {
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn a_truncated_or_foreign_parameter_set_yields_no_size() {
        assert_eq!(frame_size_from_sps("H265", &HEVC_SPS[..20]), None);
        assert_eq!(frame_size_from_sps("H264", HEVC_SPS), None);
        assert_eq!(frame_size_from_sps("VP9", HEVC_SPS), None);
        assert_eq!(frame_size_from_sps("H264", &[]), None);
    }
}
