use std::sync::Arc;
use std::time::Duration;

use futures_util::SinkExt;
use proview_core::live::{
    ClipPlan, LiveMode, LivePacket, PacketKind, StreamKey, StreamMeta, Subscription,
};
use proview_core::source::RtspTarget;
use rocket::serde::Serialize;
use rocket_ws as ws;

use crate::live_state::LiveState;

pub const FRAME_CONFIG: u8 = 1;
pub const FRAME_KEYFRAME: u8 = 2;
pub const FRAME_DELTA: u8 = 3;

#[derive(Serialize)]
struct Hello<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    mode: &'a str,
    codec: Option<&'a str>,
    width: Option<u32>,
    height: Option<u32>,
    audio: Option<&'a str>,
    audio_clock: Option<u32>,
}

pub fn requested_mode(codec: Option<&str>) -> LiveMode {
    match codec {
        Some("hevc") => LiveMode::Passthrough,
        _ => LiveMode::Transcode,
    }
}

pub fn hello(meta: &StreamMeta) -> String {
    let frame = Hello {
        kind: "hello",
        mode: LiveMode::Passthrough.as_str(),
        codec: meta.codec_string.as_deref(),
        width: meta.width,
        height: meta.height,
        audio: meta.audio_codec.as_deref(),
        audio_clock: meta.audio_clock,
    };
    serde_json::to_string(&frame).unwrap_or_else(|_| "{}".to_string())
}

fn forward(packet: &LivePacket, waiting_keyframe: &mut bool) -> bool {
    if packet.kind == PacketKind::Config || packet.kind == PacketKind::Init {
        return false;
    }
    if *waiting_keyframe {
        if !packet.keyframe {
            return false;
        }
        *waiting_keyframe = false;
    }
    true
}

pub fn frame(packet: &LivePacket) -> Vec<u8> {
    match packet.kind {
        PacketKind::AccessUnit => {
            let mut out = Vec::with_capacity(9 + packet.bytes.len());
            out.push(if packet.keyframe {
                FRAME_KEYFRAME
            } else {
                FRAME_DELTA
            });
            out.extend_from_slice(&packet.pts_us.to_be_bytes());
            out.extend_from_slice(&packet.bytes);
            out
        }
        PacketKind::Config => {
            let mut out = Vec::with_capacity(1 + packet.bytes.len());
            out.push(FRAME_CONFIG);
            out.extend_from_slice(&packet.bytes);
            out
        }
        PacketKind::Init | PacketKind::Fragment => packet.bytes.to_vec(),
    }
}

pub struct LiveSession {
    pub key: StreamKey,
    pub subscription: Subscription,
    pub restart: Option<RtspTarget>,
    pub keep_alive: Option<Arc<ClipPlan>>,
}

async fn ready(slot: &Arc<std::sync::Mutex<Option<bytes::Bytes>>>, ticks: usize) -> bool {
    for _ in 0..ticks {
        if slot.lock().map(|slot| slot.is_some()).unwrap_or(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    slot.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

pub fn channel(ws: ws::WebSocket, state: LiveState, session: LiveSession) -> ws::Channel<'static> {
    ws.channel(move |mut stream| {
        Box::pin(async move {
            let _keep_alive = session.keep_alive;
            let LiveSession {
                key,
                subscription,
                restart,
                keep_alive: _,
            } = session;
            let Subscription {
                init,
                config,
                replay,
                meta,
                mut receiver,
            } = subscription;
            let passthrough = key.mode == LiveMode::Passthrough;
            let slot = if passthrough { &config } else { &init };
            if !ready(slot, 240).await {
                if let Some(target) = restart.clone() {
                    if state.ensure_rtsp_source(key.clone(), target) {
                        ready(slot, 240).await;
                    }
                }
            }
            let ready_bytes = slot.lock().ok().and_then(|slot| slot.clone());
            match ready_bytes {
                Some(bytes) => {
                    if passthrough {
                        let now = meta.lock().map(|meta| meta.clone()).unwrap_or_default();
                        if stream.send(ws::Message::Text(hello(&now))).await.is_err() {
                            state.release(&key);
                            return Ok(());
                        }
                    }
                    let first = if passthrough {
                        LivePacket::config(bytes)
                    } else {
                        LivePacket::init(bytes)
                    };
                    if stream
                        .send(ws::Message::Binary(frame(&first)))
                        .await
                        .is_err()
                    {
                        state.release(&key);
                        return Ok(());
                    }
                }
                None => {
                    let _ = stream
                        .send(ws::Message::Text(
                            "the live source did not produce video".to_string(),
                        ))
                        .await;
                    state.release(&key);
                    let _ = stream.send(ws::Message::Close(None)).await;
                    return Ok(());
                }
            }
            let mut replayed: Option<Arc<LivePacket>> = None;
            if !passthrough {
                let held: Vec<Arc<LivePacket>> = replay
                    .lock()
                    .map(|fragments| fragments.iter().cloned().collect())
                    .unwrap_or_default();
                for packet in &held {
                    if stream
                        .send(ws::Message::Binary(frame(packet)))
                        .await
                        .is_err()
                    {
                        state.release(&key);
                        return Ok(());
                    }
                }
                replayed = held.last().cloned();
            }
            let mut waiting_keyframe = passthrough;
            loop {
                match receiver.recv().await {
                    Ok(packet) => {
                        if let Some(target) = replayed.as_ref() {
                            if Arc::ptr_eq(&packet, target) {
                                replayed = None;
                            }
                            continue;
                        }
                        if !forward(&packet, &mut waiting_keyframe) {
                            continue;
                        }
                        if stream
                            .send(ws::Message::Binary(frame(&packet)))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            skipped,
                            device = %key.device,
                            mode = key.mode.as_str(),
                            "viewer fell behind, resuming at the next keyframe"
                        );
                        replayed = None;
                        waiting_keyframe = true;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            state.release(&key);
            let _ = stream.send(ws::Message::Close(None)).await;
            Ok(())
        })
    })
}

#[derive(Serialize)]
pub struct StreamStats {
    pub device: String,
    pub channel: u32,
    pub sub_stream: bool,
    pub mode: &'static str,
    pub subscribers: usize,
    pub publishers: usize,
    pub packets: u64,
    pub keyframes: u64,
    pub bytes: u64,
    pub audio_packets: u64,
    pub drops: u64,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_clock: Option<u32>,
    pub codec_string: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Serialize)]
pub struct Stats {
    pub active_streams: usize,
    pub streams: Vec<StreamStats>,
}

pub fn snapshot(live: &LiveState) -> Stats {
    let streams: Vec<StreamStats> = live
        .hub
        .snapshots()
        .into_iter()
        .map(|snapshot| StreamStats {
            device: snapshot.device,
            channel: snapshot.channel,
            sub_stream: snapshot.sub_stream,
            mode: snapshot.mode.as_str(),
            subscribers: snapshot.subscribers,
            publishers: snapshot.publishers,
            packets: snapshot.packets,
            keyframes: snapshot.keyframes,
            bytes: snapshot.bytes,
            audio_packets: snapshot.audio_packets,
            drops: snapshot.drops,
            video_codec: snapshot.meta.video_codec,
            audio_codec: snapshot.meta.audio_codec,
            audio_clock: snapshot.meta.audio_clock,
            codec_string: snapshot.meta.codec_string,
            width: snapshot.meta.width,
            height: snapshot.meta.height,
        })
        .collect();
    Stats {
        active_streams: streams.len(),
        streams,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_core::live::Fragment;

    #[test]
    fn the_codec_query_selects_the_transport() {
        assert_eq!(requested_mode(Some("hevc")), LiveMode::Passthrough);
        assert_eq!(requested_mode(Some("avc")), LiveMode::Transcode);
        assert_eq!(requested_mode(None), LiveMode::Transcode);
    }

    #[test]
    fn the_hello_frame_carries_the_decoder_configuration() {
        let meta = StreamMeta {
            video_codec: Some("H265".to_string()),
            audio_codec: Some("PCMA".to_string()),
            audio_clock: Some(8000),
            codec_string: Some("hvc1.1.6.L63".to_string()),
            width: Some(2304),
            height: Some(2592),
        };
        let hello: serde_json::Value = serde_json::from_str(&hello(&meta)).expect("hello json");
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["mode"], "passthrough");
        assert_eq!(hello["codec"], "hvc1.1.6.L63");
        assert_eq!(hello["audio"], "PCMA");
        assert_eq!(hello["audio_clock"], 8000);
        assert_eq!(hello["width"], 2304);
        assert_eq!(hello["height"], 2592);
    }

    #[test]
    fn a_passthrough_access_unit_is_framed_with_its_timestamp() {
        let packet = LivePacket::access_unit(
            Fragment::from_static(&[0, 0, 0, 2, 0x26, 0x01]),
            true,
            1_234_567,
        );
        let bytes = frame(&packet);
        assert_eq!(bytes[0], FRAME_KEYFRAME);
        assert_eq!(&bytes[1..9], &1_234_567u64.to_be_bytes());
        assert_eq!(&bytes[9..], &[0, 0, 0, 2, 0x26, 0x01]);
        let delta = LivePacket::access_unit(Fragment::from_static(&[0, 0, 0, 1, 0x02]), false, 7);
        assert_eq!(frame(&delta)[0], FRAME_DELTA);
    }

    #[test]
    fn mse_packets_pass_through_without_a_header() {
        let init = LivePacket::init(Fragment::from_static(b"ftypmoov"));
        assert_eq!(frame(&init), b"ftypmoov".to_vec());
        let fragment = LivePacket::fragment(Fragment::from_static(b"moofmdat"));
        assert_eq!(frame(&fragment), b"moofmdat".to_vec());
        let config = LivePacket::config(Fragment::from_static(b"hvcc"));
        assert_eq!(frame(&config), b"\x01hvcc".to_vec());
    }

    #[test]
    fn a_caught_up_viewer_waits_for_the_next_keyframe() {
        let delta = LivePacket::access_unit(Fragment::from_static(&[0, 0, 0, 1, 0x02]), false, 1);
        let keyframe = LivePacket::access_unit(Fragment::from_static(&[0, 0, 0, 1, 0x26]), true, 2);
        let config = LivePacket::config(Fragment::from_static(b"hvcc"));
        let mut waiting = false;
        assert!(forward(&delta, &mut waiting));
        assert!(!forward(&config, &mut waiting));
        waiting = true;
        assert!(!forward(&delta, &mut waiting));
        assert!(waiting);
        assert!(forward(&keyframe, &mut waiting));
        assert!(!waiting);
        assert!(forward(&delta, &mut waiting));
    }
}
