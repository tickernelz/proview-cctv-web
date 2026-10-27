use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use proview_core::config::Config;
use proview_core::dvrip::talk::{TalkLink, TalkSession};
use proview_core::media::audio;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::State;
use rocket_ws as ws;
use serde::{Deserialize, Serialize};

use super::{fail, session_id, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![ws_talk]
}

#[derive(Debug, Serialize)]
pub struct TalkReady {
    pub kind: &'static str,
    pub codec: &'static str,
    pub channel: Option<u8>,
    pub rate: u32,
    pub frame: usize,
    pub claim_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct TalkReport {
    pub kind: &'static str,
    pub frames: u64,
    pub bytes: u64,
    pub chunks: u64,
    pub dropped: u64,
    pub seconds: f64,
    pub frames_per_second: f64,
    pub claim_ms: u64,
    pub write_ms_mean: f64,
    pub write_ms_max: f64,
    pub input_rate: u32,
}

#[derive(Debug, Deserialize)]
struct Control {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Default)]
struct Meter {
    chunks: u64,
    frames: u64,
    write_total: Duration,
    write_max: Duration,
}

fn address(cfg: &Config) -> Result<String, Failure> {
    cfg.device_host
        .clone()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| {
            fail(
                Status::BadRequest,
                "set PROVIEW_DEVICE_HOST to the camera's DVRIP address (host:port)",
            )
        })
}

#[get("/ws/talk?<channel>&<rate>")]
fn ws_talk(
    ws: ws::WebSocket,
    channel: Option<u8>,
    rate: Option<u32>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<ws::Channel<'static>, Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))?;
    let address = address(cfg)?;
    let user = cfg.device_user.clone();
    let password = cfg.device_pass.clone();
    let input_rate = rate.unwrap_or(audio::SAMPLE_RATE).clamp(8000, 96000);

    Ok(ws.channel(move |mut stream| {
        Box::pin(async move {
            let mut link = match TalkLink::connect(&address).await {
                Ok(link) => link,
                Err(error) => {
                    let _ = stream
                        .send(ws::Message::Text(json_error(&format!(
                            "the camera is unreachable at {address}: {error}"
                        ))))
                        .await;
                    let _ = stream.send(ws::Message::Close(None)).await;
                    return Ok(());
                }
            };
            if let Err(error) = link.login(&user, &password).await {
                let _ = stream
                    .send(ws::Message::Text(json_error(&format!(
                        "the camera refused the talk login: {error}"
                    ))))
                    .await;
                link.close().await;
                let _ = stream.send(ws::Message::Close(None)).await;
                return Ok(());
            }
            let mut talk = match TalkSession::open(link, channel).await {
                Ok(talk) => talk,
                Err(error) => {
                    let _ = stream
                        .send(ws::Message::Text(json_error(&format!(
                            "the camera refused the talk channel: {error}"
                        ))))
                        .await;
                    let _ = stream.send(ws::Message::Close(None)).await;
                    return Ok(());
                }
            };
            let ready = TalkReady {
                kind: "ready",
                codec: "g711a",
                channel,
                rate: audio::SAMPLE_RATE,
                frame: audio::FRAME_SAMPLES,
                claim_ms: talk.stats().claim_ms,
            };
            if stream
                .send(ws::Message::Text(
                    serde_json::to_string(&ready).unwrap_or_default(),
                ))
                .await
                .is_err()
            {
                let _ = talk.stop().await;
                let _ = talk.logout().await;
                return Ok(());
            }

            let mut meter = Meter::default();
            let mut last_report = Instant::now();
            while let Some(message) = stream.next().await {
                match message {
                    Ok(ws::Message::Binary(payload)) => {
                        let samples = audio::from_pcm16le(&payload);
                        let resampled = audio::downsample(&samples, input_rate, audio::SAMPLE_RATE);
                        let alaw = audio::encode(&resampled);
                        let started = Instant::now();
                        match talk.send_alaw(&alaw).await {
                            Ok(0) => {}
                            Ok(frames) => {
                                let spent = started.elapsed();
                                meter.frames += frames as u64;
                                meter.chunks += 1;
                                meter.write_total += spent;
                                meter.write_max = meter.write_max.max(spent);
                            }
                            Err(error) => {
                                let _ = stream
                                    .send(ws::Message::Text(json_error(&format!(
                                        "the camera stopped accepting audio: {error}"
                                    ))))
                                    .await;
                                break;
                            }
                        }
                        if last_report.elapsed() >= Duration::from_millis(1000) {
                            last_report = Instant::now();
                            let report = report(&talk, &meter, input_rate);
                            if stream
                                .send(ws::Message::Text(
                                    serde_json::to_string(&report).unwrap_or_default(),
                                ))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Ok(ws::Message::Text(text)) => {
                        if let Ok(control) = serde_json::from_str::<Control>(&text) {
                            if control.kind == "stop" {
                                break;
                            }
                        }
                    }
                    Ok(ws::Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }

            let final_report = report(&talk, &meter, input_rate);
            let _ = stream
                .send(ws::Message::Text(
                    serde_json::to_string(&final_report).unwrap_or_default(),
                ))
                .await;
            let _ = talk.stop().await;
            let _ = talk.logout().await;
            let _ = stream.send(ws::Message::Close(None)).await;
            Ok(())
        })
    }))
}

fn report(talk: &TalkSession, meter: &Meter, input_rate: u32) -> TalkReport {
    let stats = talk.stats();
    let mean = if meter.chunks == 0 {
        0.0
    } else {
        meter.write_total.as_secs_f64() * 1000.0 / meter.chunks as f64
    };
    TalkReport {
        kind: "report",
        frames: stats.frames,
        bytes: stats.bytes,
        chunks: meter.chunks,
        dropped: stats.dropped,
        seconds: stats.seconds,
        frames_per_second: if stats.seconds > 0.0 {
            stats.frames as f64 / stats.seconds
        } else {
            0.0
        },
        claim_ms: stats.claim_ms,
        write_ms_mean: mean,
        write_ms_max: meter.write_max.as_secs_f64() * 1000.0,
        input_rate,
    }
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "kind": "error", "message": message }).to_string()
}
