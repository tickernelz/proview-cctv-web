use std::env;
use std::time::{Duration, Instant};

use proview_core::cloud::envelope::{EnvelopeClient, Identity, MISSING_CLOUD_IDENTITY};
use proview_core::cloud::status::{auth_code_of, StatusClient, STATUS_HOST};
use proview_core::dvrip::talk::{frame_delay, TalkLink, TalkSession};
use proview_core::media::audio;
use proview_core::provider::{p2p, rps};

fn env_or(key: &str, fallback: &str) -> String {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn wav_samples(path: &str) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("{path} is not a RIFF wave file").into());
    }
    let mut offset = 12usize;
    let mut rate = 0u32;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        if id == b"fmt " && body + 8 <= bytes.len() {
            rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
        }
        if id == b"data" {
            let end = (body + size).min(bytes.len());
            let samples = audio::from_pcm16le(&bytes[body..end]);
            if rate == 0 {
                rate = audio::SAMPLE_RATE;
            }
            return Ok(audio::downsample(&samples, rate, audio::SAMPLE_RATE));
        }
        offset = body + size + size % 2;
    }
    Err(format!("{path} carries no data chunk").into())
}

async fn relay_link(
    serial_filter: Option<&str>,
) -> Result<TalkLink, Box<dyn std::error::Error>> {
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(MISSING_CLOUD_IDENTITY.into());
    }
    let account = env_or("PROVIEW_EMAIL", "");
    let password = env_or("PROVIEW_PASSWORD", "");
    if account.is_empty() || password.is_empty() {
        return Err("set PROVIEW_EMAIL and PROVIEW_PASSWORD".into());
    }
    let client = EnvelopeClient::new(identity);
    let started = Instant::now();
    let tokens = client.login_envelope(&account, &password).await?;
    eprintln!("cloud    login {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);

    let started = Instant::now();
    let devices = client.device_list(&tokens.access).await?;
    eprintln!(
        "cloud    devices {:.0} ms count={}",
        started.elapsed().as_secs_f64() * 1000.0,
        devices.len()
    );
    for device in &devices {
        eprintln!(
            "device   {} name={:?} model={:?} channels={:?} rps={}",
            device.uuid,
            device.display_name(),
            device.model,
            device.channel_num,
            device
                .extra
                .get("rps")
                .and_then(|value| value.as_str())
                .unwrap_or("")
        );
    }
    let device = match serial_filter {
        Some(serial) => devices
            .iter()
            .find(|device| device.uuid == serial)
            .ok_or_else(|| format!("serial {serial} is not on this account"))?,
        None => devices.first().ok_or("the account lists no device")?,
    };
    let serial = device.uuid.clone();
    let rps_id = device
        .extra
        .get("rps")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();

    let started = Instant::now();
    let status = StatusClient::new(STATUS_HOST)
        .query(&serial, &auth_code_of(device))
        .await?;
    eprintln!(
        "cloud    status {:.0} ms online={} server={:?}:{:?}",
        started.elapsed().as_secs_f64() * 1000.0,
        status.online,
        status.server_ip,
        status.server_port
    );
    let (access_host, access_port) = status
        .access_server()
        .ok_or("no status cluster reported an access server")?;

    let session = rps::session_id();
    let token = rps::client_token();
    let body = p2p::agent_request(
        &rps_id,
        &token,
        &serial,
        p2p::AGENT_SERVICE_MEDIA,
        &session,
        p2p::AGENT_STEP_MEDIA,
    );
    let started = Instant::now();
    let agent = rps::AccessClient::new()
        .assign(&access_host, access_port, &body)
        .await?;
    eprintln!(
        "cloud    agent {:.0} ms {}:{}",
        started.elapsed().as_secs_f64() * 1000.0,
        agent.host,
        agent.port
    );
    let frame = p2p::handshake_frame(&rps_id, &token, &serial, &session);
    let started = Instant::now();
    let tunnel = rps::open(&agent, &frame).await?;
    eprintln!(
        "cloud    tunnel {:.0} ms accepted={}",
        started.elapsed().as_secs_f64() * 1000.0,
        tunnel.reply.accepted()
    );
    Ok(TalkLink::from_stream(tunnel.stream))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let relay = matches!(env_or("PROVIEW_RELAY", "0").as_str(), "1" | "true" | "yes");
    let host = env_or("PROVIEW_DEVICE_HOST", "");
    let serial = env::var("PROVIEW_DEVICE_SERIAL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if !relay && host.is_empty() {
        return Err("set PROVIEW_DEVICE_HOST (host:port) or PROVIEW_RELAY=1".into());
    }
    let user = env_or("PROVIEW_DEVICE_USER", "admin");
    let pass = env_or("PROVIEW_DEVICE_PASS", "");
    let channel = env::var("PROVIEW_TALK_CHANNEL")
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok());
    let seconds = env::var("PROVIEW_TALK_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .unwrap_or(5.0);
    let wav = env::var("PROVIEW_TALK_WAV").ok();
    let tone_hz = env::var("PROVIEW_TALK_TONE")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .unwrap_or(1000.0);

    let samples = match wav {
        Some(path) => {
            eprintln!("source   wave {path}");
            wav_samples(&path)?
        }
        None => {
            let count = (seconds * f64::from(audio::SAMPLE_RATE)) as usize;
            eprintln!("source   tone {tone_hz} Hz for {seconds} s");
            audio::tone(tone_hz, audio::SAMPLE_RATE, count, 0.6)
        }
    };
    let alaw = audio::encode(&samples);
    eprintln!(
        "encode   {} samples -> {} A-law bytes ({} frames of {})",
        samples.len(),
        alaw.len(),
        alaw.len().div_ceil(audio::FRAME_SAMPLES),
        audio::FRAME_SAMPLES
    );

    let started = Instant::now();
    let link = if relay {
        relay_link(serial.as_deref()).await?
    } else {
        TalkLink::connect(&host).await?
    };
    let mut link = link;
    eprintln!("connect  {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    let started = Instant::now();
    link.login(&user, &pass).await?;
    eprintln!(
        "login    {:.1} ms session={}",
        started.elapsed().as_secs_f64() * 1000.0,
        link.session_id()
    );

    let mut talk = TalkSession::open(link, channel).await?;
    eprintln!(
        "talk     claim {} ms channel={:?}",
        talk.stats().claim_ms,
        talk.channel()
    );

    let started = Instant::now();
    let mut sent = 0u64;
    let mut write_total = Duration::ZERO;
    let mut write_max = Duration::ZERO;
    for chunk in alaw.chunks(audio::FRAME_SAMPLES) {
        let frame_started = Instant::now();
        talk.send_alaw(chunk).await?;
        let spent = frame_started.elapsed();
        write_total += spent;
        write_max = write_max.max(spent);
        sent += 1;
        if let Some(delay) = frame_delay(sent, started.elapsed()) {
            tokio::time::sleep(delay).await;
        }
        if sent % 25 == 0 {
            eprintln!(
                "frame    {:>4} t={:.2}s write={:.2}ms",
                sent,
                started.elapsed().as_secs_f64(),
                spent.as_secs_f64() * 1000.0
            );
        }
    }
    let elapsed = started.elapsed();
    let stats = talk.stats();
    talk.stop().await?;
    talk.logout().await?;

    println!("frames     {}", stats.frames);
    println!("bytes      {}", stats.bytes);
    println!("wall       {:.2} s", elapsed.as_secs_f64());
    println!(
        "rate       {:.1} frames/s",
        stats.frames as f64 / elapsed.as_secs_f64()
    );
    println!("claim      {} ms", stats.claim_ms);
    println!(
        "write      mean {:.3} ms max {:.3} ms",
        write_total.as_secs_f64() * 1000.0 / stats.frames as f64,
        write_max.as_secs_f64() * 1000.0
    );
    Ok(())
}
