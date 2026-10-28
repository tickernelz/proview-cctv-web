use serde_json::{json, Value};

pub const LIVE_MEDIA_ID: u16 = 0x0584;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveClass {
    GopMarker,
    KeyframeHead,
    Delta,
    Padding,
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
        0xfa => LiveClass::Padding,
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
    pub continuations: u64,
    pub elementary_bytes: u64,
    pub dropped: u64,
}

#[derive(Debug, Default)]
pub struct LiveDemux {
    pending: Option<Pending>,
    stats: LiveStats,
}

#[derive(Debug)]
struct Pending {
    declared: usize,
    payload: Vec<u8>,
}

const KEYFRAME_HEADER: usize = 16;
const DELTA_HEADER: usize = 8;
const RECORD_MAX: usize = 8 * 1024 * 1024;

impl LiveDemux {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> &LiveStats {
        &self.stats
    }

    pub fn push(&mut self, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        match classify(body) {
            LiveClass::GopMarker => {
                self.stats.markers += 1;
            }
            LiveClass::Padding => {
                self.stats.padding += 1;
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
                    declared,
                    payload: body[KEYFRAME_HEADER..].to_vec(),
                });
                self.drain_pending(&mut out);
            }
            LiveClass::Delta => {
                self.stats.deltas += 1;
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
                    self.stats.elementary_bytes += payload.len() as u64;
                    out.extend_from_slice(payload);
                } else {
                    self.flush_pending(&mut out, true);
                    self.pending = Some(Pending {
                        declared,
                        payload: body[DELTA_HEADER..].to_vec(),
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

    fn drain_pending(&mut self, out: &mut Vec<u8>) {
        let complete = self
            .pending
            .as_ref()
            .map(|pending| pending.payload.len() >= pending.declared)
            .unwrap_or(false);
        if complete {
            self.flush_pending(out, false);
        }
    }

    fn flush_pending(&mut self, out: &mut Vec<u8>, truncated: bool) {
        if let Some(mut pending) = self.pending.take() {
            if truncated && pending.payload.len() < pending.declared {
                self.stats.dropped += 1;
            }
            pending.payload.truncate(pending.declared);
            self.stats.keyframes += 1;
            self.stats.elementary_bytes += pending.payload.len() as u64;
            out.extend_from_slice(&pending.payload);
        }
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        self.flush_pending(&mut out, true);
        out
    }
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
    fn a_delta_interrupting_an_incomplete_keyframe_is_not_lost() {
        let head = keyframe_head(32, &[1, 2, 3, 4]);
        let body = delta(&[0, 0, 0, 1, 0x02, 0x01]);
        let mut demux = LiveDemux::new();
        assert!(demux.push(&head).is_empty());
        let out = demux.push(&body);
        assert_eq!(out, vec![1, 2, 3, 4, 0, 0, 0, 1, 0x02, 0x01]);
        assert_eq!(
            demux.stats().dropped,
            1,
            "the short keyframe is counted as dropped"
        );
        assert_eq!(demux.stats().keyframes, 1);
        assert_eq!(demux.stats().deltas, 1);
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
