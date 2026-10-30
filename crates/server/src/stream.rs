use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use proview_core::live::{
    ClipPlan, LiveMode, LivePacket, PacketKind, StreamKey, StreamMeta, Subscription,
};
use proview_core::source::RtspTarget;
use rocket::serde::Serialize;
use rocket_ws as ws;

use crate::live_state::LiveState;

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

pub fn served_mode(requested: LiveMode, meta: &StreamMeta) -> LiveMode {
    let hevc = meta
        .codec_string
        .as_deref()
        .is_some_and(|codec| codec.starts_with("hvc1") || codec.starts_with("hev1"));
    if requested == LiveMode::Passthrough && hevc {
        LiveMode::Passthrough
    } else {
        LiveMode::Transcode
    }
}

pub fn hello(requested: LiveMode, meta: &StreamMeta) -> String {
    let mode = served_mode(requested, meta);
    let codec = meta
        .codec_string
        .as_deref()
        .filter(|codec| mode == LiveMode::Passthrough || codec.starts_with("avc1"));
    let frame = Hello {
        kind: "hello",
        mode: mode.as_str(),
        codec,
        width: meta.width,
        height: meta.height,
        audio: meta.audio_codec.as_deref(),
        audio_clock: meta.audio_clock,
    };
    serde_json::to_string(&frame).unwrap_or_else(|_| "{}".to_string())
}

fn forward(packet: &LivePacket, waiting_keyframe: &mut bool) -> bool {
    if packet.kind == PacketKind::Init {
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

#[derive(serde::Deserialize)]
struct SoundRequest {
    #[serde(rename = "type")]
    kind: String,
    on: bool,
}

pub fn sound_request(text: &str) -> Option<bool> {
    serde_json::from_str::<SoundRequest>(text)
        .ok()
        .filter(|request| request.kind == "sound")
        .map(|request| request.on)
}

pub fn frame(packet: &LivePacket) -> Vec<u8> {
    packet.bytes.to_vec()
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
                replay,
                meta,
                mut receiver,
                mut audio,
            } = subscription;
            if !ready(&init, 240).await {
                if let Some(target) = restart.clone() {
                    if state.ensure_rtsp_source(key.clone(), target) {
                        ready(&init, 240).await;
                    }
                }
            }
            let ready_bytes = init.lock().ok().and_then(|slot| slot.clone());
            match ready_bytes {
                Some(bytes) => {
                    let now = meta.lock().map(|meta| meta.clone()).unwrap_or_default();
                    if stream
                        .send(ws::Message::Text(hello(key.mode, &now)))
                        .await
                        .is_err()
                    {
                        state.release(&key);
                        return Ok(());
                    }
                    let first = LivePacket::init(bytes);
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
            for packet in &replay {
                if stream
                    .send(ws::Message::Binary(frame(packet)))
                    .await
                    .is_err()
                {
                    state.release(&key);
                    return Ok(());
                }
            }
            let mut waiting_keyframe = replay.is_empty();
            audio = audio.resubscribe();
            let mut sound = false;
            loop {
                tokio::select! {
                    received = receiver.recv() => match received {
                        Ok(packet) => {
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
                            waiting_keyframe = true;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    heard = audio.recv() => match heard {
                        Ok(record) => {
                            if sound
                                && stream
                                    .send(ws::Message::Text(record.frame_text()))
                                    .await
                                    .is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    message = stream.next() => match message {
                        Some(Ok(ws::Message::Text(text))) => {
                            if let Some(on) = sound_request(&text) {
                                sound = on;
                            }
                        }
                        Some(Ok(ws::Message::Close(_))) | Some(Err(_)) | None => break,
                        Some(Ok(_)) => {}
                    },
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
            codec_string: Some("hvc1.1.6.L153.B0".to_string()),
            width: Some(2304),
            height: Some(2592),
        };
        let hello: serde_json::Value =
            serde_json::from_str(&hello(LiveMode::Passthrough, &meta)).expect("hello json");
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["mode"], "passthrough");
        assert_eq!(hello["codec"], "hvc1.1.6.L153.B0");
        assert_eq!(hello["audio"], "PCMA");
        assert_eq!(hello["audio_clock"], 8000);
        assert_eq!(hello["width"], 2304);
        assert_eq!(hello["height"], 2592);
    }

    #[test]
    fn a_passthrough_request_the_camera_cannot_serve_says_transcode() {
        let h264 = StreamMeta {
            video_codec: Some("H264".to_string()),
            codec_string: Some("avc1.4D401E".to_string()),
            width: Some(640),
            height: Some(720),
            ..StreamMeta::default()
        };
        let served: serde_json::Value =
            serde_json::from_str(&hello(LiveMode::Passthrough, &h264)).expect("hello json");
        assert_eq!(served["mode"], "transcode");
        assert_eq!(served["codec"], "avc1.4D401E");
        let ffmpeg = StreamMeta {
            video_codec: Some("H265".to_string()),
            ..StreamMeta::default()
        };
        assert_eq!(
            served_mode(LiveMode::Passthrough, &ffmpeg),
            LiveMode::Transcode
        );
        let camera = StreamMeta {
            codec_string: Some("hvc1.1.6.L63".to_string()),
            ..StreamMeta::default()
        };
        assert_eq!(
            served_mode(LiveMode::Passthrough, &camera),
            LiveMode::Passthrough
        );
        let transcoded: serde_json::Value =
            serde_json::from_str(&hello(LiveMode::Transcode, &camera)).expect("hello json");
        assert_eq!(transcoded["mode"], "transcode");
        assert!(
            transcoded["codec"].is_null(),
            "the camera's hvc1 string is not what a transcode socket carries"
        );
    }

    #[test]
    fn the_hello_names_the_transcode_mode_it_serves() {
        let hello: serde_json::Value =
            serde_json::from_str(&hello(LiveMode::Transcode, &StreamMeta::default()))
                .expect("hello json");
        assert_eq!(hello["mode"], "transcode");
        assert!(hello["codec"].is_null());
    }

    #[test]
    fn every_packet_is_sent_as_its_fmp4_bytes() {
        let init = LivePacket::init(Fragment::from_static(b"ftypmoov"));
        assert_eq!(frame(&init), b"ftypmoov".to_vec());
        let fragment = LivePacket::media_fragment(Fragment::from_static(b"moofmdat"), false);
        assert_eq!(frame(&fragment), b"moofmdat".to_vec());
    }

    #[test]
    fn only_a_sound_message_switches_the_audio_side_channel() {
        assert_eq!(sound_request(r#"{"type":"sound","on":true}"#), Some(true));
        assert_eq!(sound_request(r#"{"type":"sound","on":false}"#), Some(false));
        assert_eq!(sound_request(r#"{"type":"position","on":true}"#), None);
        assert_eq!(sound_request(r#"{"type":"sound"}"#), None);
        assert_eq!(sound_request("sound"), None);
    }

    #[test]
    fn a_caught_up_viewer_waits_for_the_next_keyframe() {
        let delta = LivePacket::media_fragment(Fragment::from_static(b"delta"), false);
        let keyframe = LivePacket::media_fragment(Fragment::from_static(b"key"), true);
        let init = LivePacket::init(Fragment::from_static(b"ftypmoov"));
        let mut waiting = false;
        assert!(forward(&delta, &mut waiting));
        assert!(!forward(&init, &mut waiting));
        waiting = true;
        assert!(!forward(&delta, &mut waiting));
        assert!(waiting);
        assert!(forward(&keyframe, &mut waiting));
        assert!(!waiting);
        assert!(forward(&delta, &mut waiting));
    }
}
