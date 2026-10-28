pub mod audio;
pub mod dhav;
pub mod fmp4;
pub mod h264;
pub mod hevc;
<<<<<<< HEAD
pub mod livecipher;
=======
pub mod live;
>>>>>>> 16361cc (feat(media): carry live cloud video without the media key)
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
