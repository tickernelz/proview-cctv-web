use crate::media::fmp4::{
    avcc_sample, init_segment, media_segment, Sample, TrackConfig, TIMESCALE,
};
use crate::media::h264::parse_sps;
use crate::media::nal::split_annexb;

#[derive(Debug)]
pub enum ClipError {
    NoVideo,
    NoSps,
    BadSps(String),
}

impl std::fmt::Display for ClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipError::NoVideo => write!(f, "clip has no video NAL units"),
            ClipError::NoSps => write!(f, "clip has no SPS"),
            ClipError::BadSps(e) => write!(f, "SPS rejected: {e}"),
        }
    }
}

pub fn annexb_to_fmp4(clip: &[u8], fps: u32) -> Result<Vec<u8>, ClipError> {
    let units = split_annexb(clip);
    let sps = units
        .iter()
        .find(|unit| unit.h264_type() == Some(7))
        .map(|unit| unit.data.to_vec())
        .ok_or(ClipError::NoSps)?;
    let pps = units
        .iter()
        .find(|unit| unit.h264_type() == Some(8))
        .map(|unit| unit.data.to_vec())
        .unwrap_or_default();

    let info = parse_sps(&sps).map_err(|e| ClipError::BadSps(format!("{e:?}")))?;
    let track = TrackConfig {
        width: info.width.max(16),
        height: info.height.max(16),
        sps,
        pps,
    };

    let duration = TIMESCALE / fps.max(1);
    let mut samples = Vec::new();
    let mut pending: Vec<&[u8]> = Vec::new();
    for unit in &units {
        match unit.h264_type() {
            Some(1) | Some(5) => {
                let mut data = Vec::new();
                for nal in pending.drain(..) {
                    data.extend_from_slice(&avcc_sample(nal));
                }
                data.extend_from_slice(&avcc_sample(unit.data));
                samples.push(Sample {
                    data,
                    duration,
                    keyframe: unit.h264_type() == Some(5),
                });
            }
            Some(7) | Some(8) => {}
            _ => pending.push(unit.data),
        }
    }
    if samples.is_empty() {
        return Err(ClipError::NoVideo);
    }

    let mut out = init_segment(&track);
    out.extend(media_segment(1, 0, &samples));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_without_parameter_sets_is_rejected() {
        let clip = [0, 0, 0, 1, 0x65, 0x88, 0x84];
        assert!(matches!(annexb_to_fmp4(&clip, 25), Err(ClipError::NoSps)));
    }

    #[test]
    fn a_clip_without_frames_is_rejected() {
        let mut clip = vec![0, 0, 0, 1, 0x67, 66, 0x00, 30];
        clip.extend_from_slice(&[0x80, 0x00, 0x00, 0x10, 0x00, 0x00, 0x0f, 0x00]);
        let result = annexb_to_fmp4(&clip, 25);
        assert!(matches!(
            result,
            Err(ClipError::NoVideo) | Err(ClipError::BadSps(_))
        ));
    }
}
