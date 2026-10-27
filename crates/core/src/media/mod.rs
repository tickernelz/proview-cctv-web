pub mod audio;
pub mod fmp4;
pub mod h264;
pub mod hevc;
pub mod nal;
pub mod ps;
pub mod rtp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
}

impl Codec {
    pub fn mime(self) -> &'static str {
        match self {
            Codec::H264 => "video/avc",
            Codec::H265 => "video/hevc",
        }
    }

    pub fn codec_string(self) -> &'static str {
        match self {
            Codec::H264 => "avc1.42E01E",
            Codec::H265 => "hvc1.1.6.L93.B0",
        }
    }
}
