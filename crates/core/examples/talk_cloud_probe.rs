use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use proview_core::cloud::envelope::Identity;
use proview_core::config::Config;
use proview_core::dvrip::client::Connection;
use proview_core::dvrip::msg;
use proview_core::dvrip::talk::{data_frame, talk_body};
use proview_core::media::audio;
use proview_core::media::dhav::MediaCipher;
use proview_core::provider::cloud::CloudProvider;
use proview_core::provider::{rps, Provider};
use serde_json::json;
use tokio::sync::mpsc;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn number(key: &str, fallback: f64) -> f64 {
    env(key).parse().unwrap_or(fallback)
}

fn open_json(cipher: &MediaCipher, body: &[u8]) -> Option<serde_json::Value> {
    let raw = cipher.decrypt_body(body)?;
    let text = String::from_utf8_lossy(&raw);
    serde_json::from_str::<serde_json::Value>(text.trim_end_matches(['\0', '\n', '\r']))
        .ok()
        .filter(serde_json::Value::is_object)
}

fn plain_json(body: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(
        String::from_utf8_lossy(body).trim_end_matches(['\0', '\n', '\r']),
    )
    .ok()
    .filter(serde_json::Value::is_object)
}

async fn wait_reply(
    connection: &mut Connection,
    cipher: &MediaCipher,
    wanted: u16,
    wait: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + wait;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return json!({ "error": "no reply", "seen": seen });
        }
        let Ok(Ok((header, body))) = tokio::time::timeout(left, connection.recv()).await else {
            return json!({ "error": "no reply", "seen": seen });
        };
        let opened = open_json(cipher, &body);
        let plain = plain_json(&body);
        seen.push(json!({
            "id": header.id,
            "bytes": body.len(),
            "sealed": opened.is_some(),
            "plain": plain.is_some(),
            "preview": String::from_utf8_lossy(&body)
                .trim_end_matches(['\0', '\n', '\r'])
                .chars()
                .take(200)
                .collect::<String>(),
        }));
        if header.id != wanted && header.id != wanted.wrapping_add(1) {
            continue;
        }
        if let Some(value) = opened.or(plain) {
            return value;
        }
        return json!({ "error": "reply would not open", "bytes": body.len() });
    }
}

enum Out {
    Data(Vec<u8>),
    Quit,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    if account.is_empty() || password.is_empty() {
        return Err("set PROVIEW_EMAIL and PROVIEW_PASSWORD".into());
    }
    let serial = env("PROVIEW_DEVICE_SERIAL");
    if serial.is_empty() {
        return Err("set PROVIEW_DEVICE_SERIAL".into());
    }
    let mode = {
        let raw = env("PROVIEW_TALK_MODE");
        if raw.is_empty() {
            "plain".to_string()
        } else {
            raw
        }
    };
    let channel = env("PROVIEW_TALK_CHANNEL").parse::<u8>().ok();
    let seconds = number("PROVIEW_TALK_SECONDS", 3.0);
    let mut report = json!({ "mode": mode, "serial": serial, "channel": channel });

    let started = Instant::now();
    let provider = CloudProvider::new(&Config::from_env());
    let vendor = provider.login(&account, &password).await?;
    eprintln!(
        "cloud    login {:.0} ms identity={}",
        started.elapsed().as_secs_f64() * 1000.0,
        if Identity::resolve().is_complete() {
            "complete"
        } else {
            "missing"
        }
    );
    let started = Instant::now();
    let mut connection = provider.open_talk(&vendor, &serial).await?;
    eprintln!(
        "session  {:.0} ms session=0x{:08x}",
        started.elapsed().as_secs_f64() * 1000.0,
        connection.session()
    );
    report["session_ms"] = json!(started.elapsed().as_millis() as u64);

    let session_key = rps::session_key();
    let cipher =
        MediaCipher::from_text(&session_key).ok_or("the session key is not 16 characters")?;
    let session_id = format!("0x{:08X}", connection.session());
    let sealed = mode == "sealed";
    let started = Instant::now();
    let claim = talk_body("Claim", channel, &session_id);
    if sealed {
        connection
            .send_encrypted(&cipher, 0, msg::TALK_CLAIM, &claim)
            .await?;
    } else {
        connection.send_plain(0, msg::TALK_CLAIM, &claim).await?;
    }
    let claim_ms = started.elapsed().as_millis() as u64;
    let claim_reply = wait_reply(
        &mut connection,
        &cipher,
        msg::TALK_CLAIM,
        Duration::from_secs(6),
    )
    .await;
    eprintln!("claim    {claim_ms} ms reply={claim_reply}");
    report["claim_ms"] = json!(claim_ms);
    report["claim_reply"] = claim_reply.clone();
    if claim_reply.get("Ret").and_then(|ret| ret.as_i64()) != Some(100) {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let start = talk_body("Start", channel, &session_id);
    if sealed {
        connection
            .send_encrypted(&cipher, 0, msg::TALK, &start)
            .await?;
    } else {
        connection.send_plain(0, msg::TALK, &start).await?;
    }
    eprintln!("start    sent on {}", msg::TALK);

    let alaw = audio::encode(&vec![
        0i16;
        (seconds * f64::from(audio::SAMPLE_RATE)) as usize
    ]);
    let (sender, mut queue) = mpsc::channel::<Out>(256);
    let worker = tokio::spawn(async move {
        let mut ids: BTreeMap<u16, (u64, usize, u64, u64)> = BTreeMap::new();
        let mut inbound_bytes = 0u64;
        let mut frames_out = 0u64;
        let mut bytes_out = 0u64;
        loop {
            tokio::select! {
                message = queue.recv() => {
                    match message {
                        Some(Out::Data(frame)) => {
                            let mut payload = if sealed {
                                cipher.encrypt_body(&frame).into_bytes()
                            } else {
                                frame.clone()
                            };
                            payload.push(0);
                            bytes_out += payload.len() as u64;
                            if connection
                                .send_frame_on(0, false, msg::TALK_DATA, &payload)
                                .await
                                .is_err()
                            {
                                break;
                            }
                            frames_out += 1;
                        }
                        Some(Out::Quit) | None => break,
                    }
                }
                reply = tokio::time::timeout(Duration::from_millis(250), connection.recv()) => {
                    match reply {
                        Ok(Ok((header, body))) => {
                            inbound_bytes += body.len() as u64;
                            let entry = ids.entry(header.id).or_insert((0, 0, 0, 0));
                            entry.0 += 1;
                            entry.1 = entry.1.max(body.len());
                            if open_json(&cipher, &body).is_some() {
                                entry.2 += 1;
                            }
                            if plain_json(&body).is_none() {
                                entry.3 += 1;
                            }
                        }
                        Ok(Err(_)) => break,
                        Err(_) => {}
                    }
                }
            }
        }
        (ids, inbound_bytes, frames_out, bytes_out)
    });

    let started = Instant::now();
    let mut sent = 0u64;
    for chunk in alaw.chunks(audio::FRAME_SAMPLES) {
        if sender.send(Out::Data(data_frame(chunk))).await.is_err() {
            break;
        }
        sent += 1;
        let due = Duration::from_secs_f64(
            sent as f64 * audio::FRAME_SAMPLES as f64 / f64::from(audio::SAMPLE_RATE),
        );
        if let Some(delay) = due.checked_sub(started.elapsed()) {
            tokio::time::sleep(delay).await;
        }
    }
    let elapsed = started.elapsed();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let _ = sender.send(Out::Quit).await;
    drop(sender);
    let (ids, inbound_bytes, frames_out, bytes_out) = worker.await?;

    let histogram = ids
        .iter()
        .map(|(id, (count, longest, sealed_replies, opaque))| {
            format!("{id}:{count}x{longest}b/sealed{sealed_replies}/opaque{opaque}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "sent       {sent} chunks ready, worker wrote {frames_out} frames / {bytes_out} bytes"
    );
    println!("wall       {:.2} s", elapsed.as_secs_f64());
    println!("inbound    {inbound_bytes} bytes · {histogram}");
    report["frames_out"] = json!(frames_out);
    report["bytes_out"] = json!(bytes_out);
    report["seconds"] = json!(elapsed.as_secs_f64());
    report["inbound_bytes"] = json!(inbound_bytes);
    report["inbound_ids"] = json!(histogram);
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
