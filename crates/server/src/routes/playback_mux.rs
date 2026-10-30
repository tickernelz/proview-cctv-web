use std::process::Stdio;

use bytes::Bytes;
use proview_core::media::fmp4::Sample;
use proview_core::media::mux::{
    is_keyframe, sample_data, split_annexb, FragmentMux, Ticks, VideoConfig,
};
use proview_core::media::Codec;
use proview_core::source::{Fmp4Splitter, Piece};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub(crate) const AVC_MIME: &str = "video/mp4; codecs=\"avc1.42E01E\"";
const FRAGMENT_FRAMES: usize = 12;
const PIPE_DEPTH: usize = 64;

pub(crate) fn demuxer(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "h264",
        Codec::H265 => "hevc",
    }
}

pub(crate) fn wants_native(codec: Codec, client: Option<&str>) -> bool {
    match codec {
        Codec::H264 => true,
        Codec::H265 => client == Some("hevc"),
    }
}

pub(crate) fn config_of(codec: Codec, units: &[Vec<u8>]) -> Option<VideoConfig> {
    units
        .iter()
        .find_map(|unit| VideoConfig::from_nals(codec, &split_annexb(unit)).ok())
        .filter(|config| !config.reorders)
}

pub(crate) fn ffmpeg(codec: Codec, fps: Option<f64>) -> Command {
    let binary = std::env::var("PROVIEW_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let mut command = Command::new(binary);
    proview_core::process::hide_console(command.as_std_mut());
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-flags",
        "low_delay",
        "-probesize",
        "32768",
        "-analyzeduration",
        "0",
        "-f",
        demuxer(codec),
    ]);
    if let Some(fps) = fps {
        command.args(["-framerate", &format!("{fps:.3}")]);
    }
    command.args([
        "-i",
        "pipe:0",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-tune",
        "zerolatency",
        "-profile:v",
        "baseline",
        "-g",
        "25",
        "-keyint_min",
        "25",
        "-sc_threshold",
        "0",
        "-an",
        "-f",
        "mp4",
        "-movflags",
        "+empty_moov+default_base_moof+frag_keyframe+separate_moof+omit_tfhd_offset",
        "pipe:1",
    ]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

pub(crate) struct NativeSink {
    codec: Codec,
    mux: FragmentMux,
    ticks: Ticks,
    fps: f64,
    started: bool,
    sender: mpsc::Sender<Piece>,
}

pub(crate) enum Sink {
    Native(NativeSink),
    Ffmpeg(ChildStdin),
}

impl Sink {
    pub(crate) async fn unit(&mut self, annexb: &[u8]) -> bool {
        match self {
            Sink::Ffmpeg(stdin) => stdin.write_all(annexb).await.is_ok(),
            Sink::Native(sink) => sink.unit(annexb).await,
        }
    }

    pub(crate) async fn finish(self) {
        match self {
            Sink::Ffmpeg(mut stdin) => {
                let _ = stdin.shutdown().await;
            }
            Sink::Native(mut sink) => {
                if let Some((segment, _)) = sink.mux.flush() {
                    let _ = sink
                        .sender
                        .send(Piece::Fragment(Bytes::from(segment)))
                        .await;
                }
            }
        }
    }
}

impl NativeSink {
    async fn unit(&mut self, annexb: &[u8]) -> bool {
        let nals = split_annexb(annexb);
        let keyframe = is_keyframe(self.codec, &nals);
        if keyframe {
            self.started = true;
        }
        if !self.started {
            return true;
        }
        let data = sample_data(self.codec, &nals);
        if data.is_empty() {
            return true;
        }
        let duration = self.ticks.next(self.fps);
        for (segment, _) in self.mux.push(Sample {
            data,
            duration,
            keyframe,
        }) {
            if self
                .sender
                .send(Piece::Fragment(Bytes::from(segment)))
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }
}

pub(crate) enum Output {
    Native {
        init: Option<Bytes>,
        receiver: mpsc::Receiver<Piece>,
    },
    Ffmpeg {
        stdout: ChildStdout,
        splitter: Fmp4Splitter,
        ready: std::collections::VecDeque<Piece>,
    },
}

impl Output {
    pub(crate) async fn next(&mut self) -> Option<Piece> {
        match self {
            Output::Native { init, receiver } => match init.take() {
                Some(bytes) => Some(Piece::Init(bytes)),
                None => receiver.recv().await,
            },
            Output::Ffmpeg {
                stdout,
                splitter,
                ready,
            } => {
                let mut buffer = vec![0u8; 64 * 1024];
                loop {
                    if let Some(piece) = ready.pop_front() {
                        return Some(piece);
                    }
                    let read = match stdout.read(&mut buffer).await {
                        Ok(0) | Err(_) => return None,
                        Ok(read) => read,
                    };
                    ready.extend(splitter.push(&buffer[..read]));
                }
            }
        }
    }
}

pub(crate) struct Pipeline {
    pub sink: Sink,
    pub output: Output,
    pub child: Option<Child>,
    pub stderr: Option<JoinHandle<String>>,
}

pub(crate) fn native(codec: Codec, config: &VideoConfig, fps: f64) -> Pipeline {
    let (sender, receiver) = mpsc::channel(PIPE_DEPTH);
    Pipeline {
        sink: Sink::Native(NativeSink {
            codec,
            mux: FragmentMux::new(FRAGMENT_FRAMES),
            ticks: Ticks::default(),
            fps,
            started: false,
            sender,
        }),
        output: Output::Native {
            init: Some(Bytes::from(config.fragmented_init())),
            receiver,
        },
        child: None,
        stderr: None,
    }
}

pub(crate) fn transcode(codec: Codec, fps: Option<f64>) -> Result<Pipeline, String> {
    let mut child = ffmpeg(codec, fps)
        .spawn()
        .map_err(|error| format!("ffmpeg did not start: {error}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ffmpeg did not start: no stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ffmpeg did not start: no stdout".to_string())?;
    let stderr = child.stderr.take().map(|mut handle| {
        tokio::spawn(async move {
            let mut tail = String::new();
            let mut chunk = [0u8; 2048];
            while let Ok(read) = handle.read(&mut chunk).await {
                if read == 0 {
                    break;
                }
                tail.push_str(&String::from_utf8_lossy(&chunk[..read]));
                if tail.len() > 4096 {
                    tail = tail.chars().skip(tail.chars().count() - 2048).collect();
                }
            }
            tail
        })
    });
    Ok(Pipeline {
        sink: Sink::Ffmpeg(stdin),
        output: Output::Ffmpeg {
            stdout,
            splitter: Fmp4Splitter::new(),
            ready: std::collections::VecDeque::new(),
        },
        child: Some(child),
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_core::dvrip::playback::{Record, RecordKind};
    use proview_core::media::mux::ClipMux;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }

    fn keyframe() -> Vec<u8> {
        annexb(&[
            hex("40010c01ffff0160000003000003000003000003003f2c0c00000fa00000bb8140"),
            hex("4201010160000003000003000003000003003fa001202002887f9cbabc12e080"),
            hex("4401c0f3c0e14524"),
            vec![0x26, 0x01, 0xaf, 0x10],
        ])
    }

    fn delta() -> Vec<u8> {
        annexb(&[vec![0x02, 0x01, 0xd0, 0x20]])
    }

    fn packed(second: u32) -> u32 {
        (26u32 << 26) | (10 << 22) | (24 << 17) | (10 << 12) | (34 << 6) | second
    }

    #[test]
    fn only_hevc_waits_for_the_client_to_ask() {
        assert!(wants_native(Codec::H264, None));
        assert!(wants_native(Codec::H264, Some("avc")));
        assert!(wants_native(Codec::H265, Some("hevc")));
        assert!(!wants_native(Codec::H265, None));
        assert!(!wants_native(Codec::H265, Some("avc")));
    }

    #[test]
    fn the_config_is_read_from_the_first_unit_that_carries_it() {
        let config = config_of(Codec::H265, &[delta(), keyframe()]).expect("config");
        assert_eq!((config.width, config.height), (2304, 2592));
        assert!(config_of(Codec::H265, &[delta()]).is_none());
        let b_frames = annexb(&[
            hex("40010c01ffff01600000030090000003000003003c959809"),
            hex("42010101600000030090000003000003003ca00a080f165959a4932bc05a020000030002000003003210"),
            hex("4401c172b46240"),
            vec![0x26, 0x01, 0xaf, 0x10],
        ]);
        assert!(
            config_of(Codec::H265, &[b_frames]).is_none(),
            "a stream with B-frames goes through the transcode"
        );
    }

    #[tokio::test]
    async fn a_native_clip_sends_its_init_then_fragments_without_a_process() {
        let config = config_of(Codec::H265, &[keyframe()]).expect("config");
        let Pipeline {
            mut sink,
            mut output,
            child,
            stderr,
        } = native(Codec::H265, &config, 12.5);
        assert!(child.is_none(), "the native path must not spawn ffmpeg");
        assert!(stderr.is_none());
        let feeder = tokio::spawn(async move {
            assert!(
                sink.unit(&delta()).await,
                "a delta before the first keyframe is skipped"
            );
            for index in 0..30 {
                let unit = if index % 25 == 0 { keyframe() } else { delta() };
                assert!(sink.unit(&unit).await);
            }
            sink.finish().await;
        });
        let mut pieces = Vec::new();
        while let Some(piece) = output.next().await {
            pieces.push(piece);
        }
        feeder.await.expect("feeder");
        match &pieces[0] {
            Piece::Init(bytes) => assert!(bytes.windows(4).any(|window| window == b"hvcC")),
            Piece::Fragment(_) => panic!("the init comes first"),
        }
        let samples: u32 = pieces[1..]
            .iter()
            .map(|piece| match piece {
                Piece::Fragment(bytes) => {
                    let at = bytes
                        .windows(4)
                        .position(|window| window == b"trun")
                        .expect("trun");
                    u32::from_be_bytes([
                        bytes[at + 8],
                        bytes[at + 9],
                        bytes[at + 10],
                        bytes[at + 11],
                    ])
                }
                Piece::Init(_) => panic!("one init only"),
            })
            .sum();
        assert_eq!(samples, 30);
        assert_eq!(pieces.len(), 1 + 3 + 1);
    }

    fn record(kind: RecordKind, stamp: Option<u32>, bytes: Vec<u8>) -> Record {
        Record {
            kind,
            stamp,
            rate: if kind == RecordKind::Key {
                Some(12)
            } else {
                None
            },
            bytes,
        }
    }

    #[test]
    fn a_downloaded_clip_is_an_mp4_timed_by_the_record_stamps() {
        let mut records = vec![record(RecordKind::Delta, None, delta())];
        for gop in 0..10u32 {
            records.push(record(RecordKind::Key, Some(packed(gop * 2)), keyframe()));
            records.push(record(RecordKind::Pad, None, Vec::new()));
            for _ in 1..25 {
                records.push(record(RecordKind::Delta, None, delta()));
            }
        }
        let mut mux = ClipMux::new(1 << 20);
        for record in records {
            mux.push(record);
        }
        let clip = mux.finish().expect("mp4");
        assert!(clip.muxed);
        assert_eq!(clip.codec, Codec::H265);
        assert_eq!(clip.frames, 250);
        assert_eq!(clip.fps, 12.5);
        assert_eq!(clip.fps_source, "stamps");
        assert!((clip.seconds - 20.0).abs() < 1e-9);
        assert_eq!(&clip.bytes[4..8], b"ftyp");
        let mvhd = clip
            .bytes
            .windows(4)
            .position(|window| window == b"mvhd")
            .expect("mvhd");
        let duration =
            u32::from_be_bytes(clip.bytes[mvhd + 20..mvhd + 24].try_into().expect("u32"));
        assert_eq!(duration, 20_000);
    }
}
