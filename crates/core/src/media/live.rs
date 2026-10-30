use std::collections::VecDeque;

use base64::Engine;
use serde_json::{json, Value};

use crate::media::livecipher::LiveCipher;

pub const LIVE_MEDIA_ID: u16 = 0x0584;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveClass {
    GopMarker,
    KeyframeHead,
    Delta,
    Audio,
    Continuation,
    Unknown,
}

pub fn classify(body: &[u8]) -> LiveClass {
    if body.len() < 4 || body[0] != 0 || body[1] != 0 || body[2] != 1 {
        return LiveClass::Continuation;
    }
    match body[3] {
        0xf9 => LiveClass::GopMarker,
        0xfc => LiveClass::KeyframeHead,
        0xfd => LiveClass::Delta,
        0xfa => LiveClass::Audio,
        nal if nal <= 0x7f => LiveClass::Continuation,
        _ => LiveClass::Unknown,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LiveStats {
    pub keyframes: u64,
    pub deltas: u64,
    pub markers: u64,
    pub padding: u64,
    pub audio: u64,
    pub continuations: u64,
    pub elementary_bytes: u64,
    pub dropped: u64,
}

#[derive(Debug, Default)]
pub struct LiveDemux {
    pending: Option<Pending>,
    cipher: Option<LiveCipher>,
    stats: LiveStats,
    audio: VecDeque<LiveAudio>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveAudio {
    pub codec: &'static str,
    pub rate: u32,
    pub frame: u64,
    pub bytes: Vec<u8>,
}

impl LiveAudio {
    pub fn frame_text(&self) -> String {
        json!({
            "type": "audio",
            "frame": self.frame,
            "codec": self.codec,
            "rate": self.rate,
            "data": base64::engine::general_purpose::STANDARD.encode(&self.bytes),
        })
        .to_string()
    }
}

pub const AUDIO_HEADER: usize = 8;
pub const AUDIO_FIXED_SAMPLES: usize = 320;
const AUDIO_QUEUE: usize = 256;

pub fn audio_codec(code: u8) -> Option<&'static str> {
    match code {
        0x0e => Some("alaw"),
        0x0a => Some("ulaw"),
        _ => None,
    }
}

pub fn audio_rate(code: u8) -> Option<u32> {
    match code {
        0x02 => Some(8000),
        _ => None,
    }
}

pub fn audio_record(body: &[u8], frame: u64) -> Option<LiveAudio> {
    if body.len() < AUDIO_HEADER || body[..4] != [0, 0, 1, 0xfa] {
        return None;
    }
    let codec = audio_codec(body[4])?;
    let rate = audio_rate(body[5])?;
    let declared = match u16::from_le_bytes([body[6], body[7]]) as usize {
        0 => AUDIO_FIXED_SAMPLES,
        length => length,
    };
    let bytes = body.get(AUDIO_HEADER..AUDIO_HEADER + declared)?;
    Some(LiveAudio {
        codec,
        rate,
        frame,
        bytes: bytes.to_vec(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Keyframe,
    Delta,
}

#[derive(Debug)]
struct Pending {
    kind: PendingKind,
    declared: usize,
    payload: Vec<u8>,
    rate: Option<u8>,
    stamp: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveFrame {
    pub keyframe: bool,
    pub rate: Option<u8>,
    pub stamp: Option<u32>,
    pub bytes: Vec<u8>,
}

const KEYFRAME_HEADER: usize = 16;
const DELTA_HEADER: usize = 8;
const RECORD_MAX: usize = 8 * 1024 * 1024;

impl LiveDemux {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn keyed(material: &str) -> Option<Self> {
        let cipher = LiveCipher::derive(material)?;
        Some(Self {
            cipher: Some(cipher),
            ..Self::default()
        })
    }

    pub fn stats(&self) -> &LiveStats {
        &self.stats
    }

    pub fn take_audio(&mut self) -> Vec<LiveAudio> {
        self.audio.drain(..).collect()
    }

    pub fn push(&mut self, body: &[u8]) -> Vec<u8> {
        join(self.push_frames(body))
    }

    pub fn push_frames(&mut self, body: &[u8]) -> Vec<LiveFrame> {
        let mut out = Vec::new();
        match classify(body) {
            LiveClass::GopMarker => {
                self.stats.markers += 1;
            }
            LiveClass::Audio => {
                let frame = self.stats.keyframes + self.stats.deltas;
                match audio_record(body, frame) {
                    Some(audio) => {
                        self.stats.audio += 1;
                        if self.audio.len() == AUDIO_QUEUE {
                            self.audio.pop_front();
                        }
                        self.audio.push_back(audio);
                    }
                    None => self.stats.padding += 1,
                }
            }
            LiveClass::KeyframeHead => {
                self.flush_pending(&mut out, true);
                if body.len() < KEYFRAME_HEADER {
                    self.stats.dropped += 1;
                    return out;
                }
                let declared =
                    u32::from_le_bytes([body[12], body[13], body[14], body[15]]) as usize;
                if declared > RECORD_MAX {
                    self.stats.dropped += 1;
                    return out;
                }
                self.pending = Some(Pending {
                    kind: PendingKind::Keyframe,
                    declared,
                    payload: body[KEYFRAME_HEADER..].to_vec(),
                    rate: Some(body[5]).filter(|fps| *fps > 0),
                    stamp: Some(u32::from_le_bytes([body[8], body[9], body[10], body[11]])),
                });
                self.drain_pending(&mut out);
            }
            LiveClass::Delta => {
                if body.len() <= DELTA_HEADER {
                    self.stats.dropped += 1;
                    return out;
                }
                let declared = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
                if declared > RECORD_MAX {
                    self.flush_pending(&mut out, true);
                    self.stats.dropped += 1;
                    return out;
                }
                if body.len() - DELTA_HEADER >= declared {
                    self.flush_pending(&mut out, true);
                    let payload = &body[DELTA_HEADER..DELTA_HEADER + declared];
                    self.stats.deltas += 1;
                    self.stats.elementary_bytes += payload.len() as u64;
                    out.push(LiveFrame {
                        keyframe: false,
                        rate: None,
                        stamp: None,
                        bytes: payload.to_vec(),
                    });
                } else {
                    self.flush_pending(&mut out, true);
                    self.pending = Some(Pending {
                        kind: PendingKind::Delta,
                        declared,
                        payload: body[DELTA_HEADER..].to_vec(),
                        rate: None,
                        stamp: None,
                    });
                }
            }
            LiveClass::Continuation => match self.pending.as_mut() {
                Some(pending) => {
                    self.stats.continuations += 1;
                    pending.payload.extend_from_slice(body);
                    self.drain_pending(&mut out);
                }
                None => {
                    self.stats.dropped += 1;
                }
            },
            LiveClass::Unknown => {
                self.stats.dropped += 1;
            }
        }
        out
    }

    fn drain_pending(&mut self, out: &mut Vec<LiveFrame>) {
        let complete = self
            .pending
            .as_ref()
            .map(|pending| pending.payload.len() >= pending.declared)
            .unwrap_or(false);
        if complete {
            self.flush_pending(out, false);
        }
    }

    fn flush_pending(&mut self, out: &mut Vec<LiveFrame>, truncated: bool) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        if truncated && pending.payload.len() < pending.declared {
            self.stats.dropped += 1;
            return;
        }
        pending.payload.truncate(pending.declared);
        match pending.kind {
            PendingKind::Keyframe => {
                if let Some(cipher) = &self.cipher {
                    cipher.mask(&mut pending.payload, pending.declared as u32);
                }
                self.stats.keyframes += 1;
            }
            PendingKind::Delta => {
                self.stats.deltas += 1;
            }
        }
        self.stats.elementary_bytes += pending.payload.len() as u64;
        out.push(LiveFrame {
            keyframe: pending.kind == PendingKind::Keyframe,
            rate: pending.rate,
            stamp: pending.stamp,
            bytes: pending.payload,
        });
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        self.flush_pending(&mut out, true);
        join(out)
    }
}

fn join(frames: Vec<LiveFrame>) -> Vec<u8> {
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Vec::new();
    };
    let mut out = first.bytes;
    for frame in frames {
        out.extend_from_slice(&frame.bytes);
    }
    out
}

pub fn start_body(session: u32, channel: u8, stream: &str) -> Value {
    json!({
        "Name": "OPMonitor",
        "OPMonitor": {
            "Action": "Start",
            "Parameter": {
                "Channel": channel,
                "CombinMode": "NONE",
                "StreamType": stream,
                "TransMode": "TCP",
            },
        },
        "SessionID": format!("0x{session:08x}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyframe_head(declared: usize, payload: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8; 16];
        body[0] = 0;
        body[1] = 0;
        body[2] = 1;
        body[3] = 0xfc;
        body[12..16].copy_from_slice(&(declared as u32).to_le_bytes());
        body.extend_from_slice(payload);
        body
    }

    fn delta(payload: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8; 8];
        body[2] = 1;
        body[3] = 0xfd;
        body[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(payload);
        body
    }

    #[test]
    fn frames_keep_their_boundaries_rate_and_stamp() {
        let mut demux = LiveDemux::new();
        let mut head = keyframe_head(6, &[0, 0, 0, 1, 0x26, 0x01]);
        head[5] = 12;
        head[8..12].copy_from_slice(&0x6ab3_6607u32.to_le_bytes());
        let key = demux.push_frames(&head);
        assert_eq!(
            key,
            vec![LiveFrame {
                keyframe: true,
                rate: Some(12),
                stamp: Some(0x6ab3_6607),
                bytes: vec![0, 0, 0, 1, 0x26, 0x01],
            }]
        );
        let frames = demux.push_frames(&delta(&[0, 0, 0, 1, 0x02, 0x01]));
        assert_eq!(frames.len(), 1);
        assert!(!frames[0].keyframe);
        assert_eq!(frames[0].rate, None);
        let mut joined = LiveDemux::new();
        let mut split = keyframe_head(6, &[0, 0, 0, 1]);
        split[5] = 12;
        assert!(joined.push_frames(&split).is_empty());
        let both = joined.push_frames(&delta(&[0, 0, 0, 1, 0x02]));
        assert_eq!(
            both.len(),
            1,
            "an incomplete keyframe is dropped, the delta kept"
        );
        let mut whole = LiveDemux::new();
        assert!(whole.push_frames(&split).is_empty());
        let completed = whole.push_frames(&[0x26, 0x01]);
        assert_eq!(completed.len(), 1);
        assert!(completed[0].keyframe);
        assert_eq!(completed[0].rate, Some(12));
    }

    #[test]
    fn a_keyframe_split_over_continuations_emits_exactly_the_declared_payload() {
        let head = keyframe_head(24, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut demux = LiveDemux::new();
        let mut out = demux.push(&head);
        assert!(
            out.is_empty(),
            "an incomplete keyframe waits for its continuations"
        );
        out = demux.push(&[9u8, 10, 11, 12, 13, 14, 15, 16]);
        assert!(out.is_empty(), "still short of the declared length");
        out = demux.push(&[17u8, 18, 19, 20, 21, 22, 23, 24, 25, 26]);
        assert_eq!(
            out,
            (1..=24u8).collect::<Vec<u8>>(),
            "the extra bytes are cut at the declared length"
        );
        assert_eq!(demux.stats().keyframes, 1);
        assert_eq!(demux.stats().continuations, 2);
    }

    #[test]
    fn a_delta_emits_its_payload_with_the_inner_start_code_intact() {
        let body = delta(&[0, 0, 0, 1, 0x02, 0x01, 0xd0]);
        let mut demux = LiveDemux::new();
        let out = demux.push(&body);
        assert_eq!(out, vec![0, 0, 0, 1, 0x02, 0x01, 0xd0]);
        assert_eq!(demux.stats().deltas, 1);
    }

    #[test]
    fn padding_and_gop_markers_never_reach_the_elementary_stream() {
        let mut demux = LiveDemux::new();
        let mut padding = vec![0u8; 8];
        padding[2] = 1;
        padding[3] = 0xfa;
        let mut marker = vec![0u8; 16];
        marker[2] = 1;
        marker[3] = 0xf9;
        assert!(demux.push(&padding).is_empty());
        assert!(demux.push(&marker).is_empty());
        assert_eq!(demux.stats().padding, 1);
        assert_eq!(demux.stats().markers, 1);
        assert_eq!(demux.stats().elementary_bytes, 0);
    }

    #[test]
    fn a_delta_interrupting_an_incomplete_keyframe_keeps_only_the_delta() {
        let head = keyframe_head(32, &[1, 2, 3, 4]);
        let body = delta(&[0, 0, 0, 1, 0x02, 0x01]);
        let mut demux = LiveDemux::new();
        assert!(demux.push(&head).is_empty());
        let out = demux.push(&body);
        assert_eq!(out, vec![0, 0, 0, 1, 0x02, 0x01]);
        assert_eq!(
            demux.stats().dropped,
            1,
            "the short keyframe is dropped whole, never half-emitted"
        );
        assert_eq!(demux.stats().keyframes, 0);
        assert_eq!(demux.stats().deltas, 1);
    }

    fn audio(fill: u8) -> Vec<u8> {
        let mut body = vec![0, 0, 1, 0xfa, 0x0e, 0x02, 0x40, 0x01];
        body.extend([fill; 320]);
        body
    }

    #[test]
    fn an_audio_record_is_queued_and_the_video_around_it_is_unchanged() {
        let head = keyframe_head(6, &[0, 0, 0, 1, 0x26, 0x01]);
        let body = delta(&[0, 0, 0, 1, 0x02, 0x01]);
        let mut plain = LiveDemux::new();
        let mut expected = plain.push_frames(&head);
        expected.extend(plain.push_frames(&body));

        let mut demux = LiveDemux::new();
        assert!(demux.push_frames(&audio(0xd5)).is_empty());
        let mut frames = demux.push_frames(&head);
        assert!(demux.push_frames(&audio(0x55)).is_empty());
        frames.extend(demux.push_frames(&body));
        assert_eq!(frames, expected);

        let heard = demux.take_audio();
        assert_eq!(heard.len(), 2);
        assert_eq!(heard[0].codec, "alaw");
        assert_eq!(heard[0].rate, 8000);
        assert_eq!(heard[0].frame, 0);
        assert_eq!(heard[0].bytes, vec![0xd5u8; 320]);
        assert_eq!(
            heard[1].frame, 1,
            "the second record follows one video frame"
        );
        assert_eq!(heard[1].bytes, vec![0x55u8; 320]);
        assert!(demux.take_audio().is_empty());
        assert_eq!(demux.stats().audio, 2);
        assert_eq!(demux.stats().padding, 0);
        assert_eq!(demux.stats().elementary_bytes, 12);
    }

    #[test]
    fn a_zero_length_audio_record_holds_the_fixed_frame_and_a_short_one_is_padding() {
        let mut fixed = audio(0xd5);
        fixed[6] = 0;
        fixed[7] = 0;
        let mut demux = LiveDemux::new();
        demux.push_frames(&fixed);
        assert_eq!(demux.take_audio()[0].bytes.len(), 320);
        let mut short = audio(0xd5);
        short.truncate(100);
        let mut unknown = audio(0xd5);
        unknown[4] = 0x63;
        demux.push_frames(&short);
        demux.push_frames(&unknown);
        assert!(demux.take_audio().is_empty());
        assert_eq!(demux.stats().audio, 1);
        assert_eq!(demux.stats().padding, 2);
    }

    #[test]
    fn the_audio_queue_keeps_only_the_newest_records() {
        let mut demux = LiveDemux::new();
        for _ in 0..AUDIO_QUEUE + 10 {
            demux.push_frames(&audio(0xd5));
        }
        assert_eq!(demux.take_audio().len(), AUDIO_QUEUE);
    }

    #[test]
    fn an_audio_frame_uses_the_playback_side_channel_shape() {
        let record = audio_record(&audio(0xd5), 7).expect("audio");
        let text: Value = serde_json::from_str(&record.frame_text()).expect("json");
        assert_eq!(text["type"], "audio");
        assert_eq!(text["frame"], 7);
        assert_eq!(text["codec"], "alaw");
        assert_eq!(text["rate"], 8000);
        let data = base64::engine::general_purpose::STANDARD
            .decode(text["data"].as_str().expect("data"))
            .expect("base64");
        assert_eq!(data, vec![0xd5u8; 320]);
    }

    #[test]
    fn a_keyed_demux_unmasks_the_head_of_a_keyframe_and_leaves_a_delta_alone() {
        let material = "zha4084-fixture-material-0000000001";
        let cipher = LiveCipher::derive(material).expect("material");
        let plain: Vec<u8> = (0..300u32).map(|i| ((i * 5 + 3) & 0xff) as u8).collect();
        let mut masked = plain.clone();
        cipher.mask(&mut masked, 300);
        assert_ne!(&masked[..128], &plain[..128]);
        assert_eq!(&masked[128..], &plain[128..]);

        let head = keyframe_head(300, &masked);
        let body = delta(&[0, 0, 0, 1, 0x02, 0x01, 0xd0]);
        let mut demux = LiveDemux::keyed(material).expect("keyed demux");
        let mut out = demux.push(&head);
        out.extend(demux.push(&body));
        assert_eq!(out[..300], plain[..]);
        assert_eq!(&out[300..], &[0, 0, 0, 1, 0x02, 0x01, 0xd0]);
        assert_eq!(demux.stats().keyframes, 1);
        assert_eq!(demux.stats().deltas, 1);
        assert_eq!(demux.stats().elementary_bytes, 307);
    }

    #[test]
    fn a_keyed_demux_without_the_material_stays_plain() {
        assert!(LiveDemux::keyed("too short").is_none());
        let plain = [0u8, 0, 1, 0x40, 0x01, 0x0c];
        let head = keyframe_head(plain.len(), &plain);
        let mut demux = LiveDemux::new();
        assert_eq!(demux.push(&head), plain.to_vec());
    }

    #[test]
    fn a_truncated_keyframe_at_finish_is_dropped_whole() {
        let head = keyframe_head(64, &[9, 8, 7, 6]);
        let mut demux = LiveDemux::new();
        assert!(demux.push(&head).is_empty());
        assert!(demux.finish().is_empty());
        assert_eq!(demux.stats().dropped, 1);
        assert_eq!(demux.stats().keyframes, 0);
        assert_eq!(demux.stats().elementary_bytes, 0);
    }

    #[test]
    fn a_continuation_without_an_open_keyframe_is_dropped_and_counted() {
        let mut demux = LiveDemux::new();
        let body = vec![7u8; 8192];
        assert!(demux.push(&body).is_empty());
        assert_eq!(demux.stats().dropped, 1);
        assert_eq!(demux.stats().elementary_bytes, 0);
    }

    #[test]
    fn the_start_body_never_carries_a_media_key() {
        let body = start_body(0x2d3, 0, "Main");
        let parameter = &body["OPMonitor"]["Parameter"];
        assert_eq!(body["OPMonitor"]["Action"], "Start");
        assert_eq!(parameter["CombinMode"], "NONE");
        assert_eq!(parameter["StreamType"], "Main");
        assert_eq!(parameter["TransMode"], "TCP");
        assert_eq!(parameter["Channel"], 0);
        assert!(
            parameter.get("VEKEY1").is_none(),
            "a VEKEY1 turns the media cipher back on"
        );
        assert_eq!(body["SessionID"], "0x000002d3");
    }
    #[test]
    fn a_delta_larger_than_one_body_is_joined_by_its_declared_length() {
        let payload = vec![0x11u8; 20_000];
        let mut first = Vec::new();
        first.extend_from_slice(&[0, 0, 1, 0xfd]);
        first.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        first.extend_from_slice(&payload[..8_184]);
        let second = payload[8_184..16_368].to_vec();
        let tail = &payload[16_368..];
        let mut demux = LiveDemux::new();
        let mut out = demux.push(&first);
        out.extend(demux.push(&second));
        out.extend(demux.push(tail));
        assert_eq!(out.len(), payload.len());
        assert_eq!(demux.stats().dropped, 0);
        assert_eq!(demux.stats().deltas, 1);
        assert_eq!(demux.stats().keyframes, 0);
    }

    #[test]
    fn a_continuation_that_starts_like_a_start_code_is_not_dropped() {
        let head = {
            let mut body = Vec::new();
            body.extend_from_slice(&[0, 0, 1, 0xfc]);
            body.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
            body.extend_from_slice(&(16u32).to_le_bytes());
            body.extend_from_slice(&[0xaa; 8]);
            body
        };
        let continuation = [0u8, 0, 1, 0x26, 0xbb, 0xcc, 0xdd, 0xee];
        let mut demux = LiveDemux::new();
        let mut out = demux.push(&head);
        out.extend(demux.push(&continuation));
        assert_eq!(out.len(), 16);
        assert_eq!(demux.stats().dropped, 0);
    }
}
