use std::time::{Duration, Instant};

use proview_core::media::{hevc, rtp};
use proview_core::rtsp::RtspSession;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let target = args
        .next()
        .unwrap_or_else(|| "192.168.0.100:554".to_string());
    let seconds: u64 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(6);
    let out = args.next().unwrap_or_else(|| "live.h265".to_string());
    let path = env("PROVIEW_RTSP_PATH");
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        path
    };
    let user = env("PROVIEW_DEVICE_USER");
    let password = env("PROVIEW_DEVICE_PASS");

    let (host, port) = match target.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse::<u16>().unwrap_or(554)),
        None => (target.clone(), 554),
    };

    let started = Instant::now();
    let mut session = match RtspSession::open(&host, port, &path, &user, &password).await {
        Ok(session) => session,
        Err(error) => {
            println!("{{\"ok\":false,\"stage\":\"describe\",\"error\":\"{error}\"}}");
            std::process::exit(1);
        }
    };
    let track = session.video_track().cloned();
    let (codec, framerate) = match &track {
        Some(track) => (track.codec.clone(), track.framerate.unwrap_or(0)),
        None => (String::new(), 0),
    };
    if let Err(error) = session.setup_video((0, 1)).await {
        println!("{{\"ok\":false,\"stage\":\"setup\",\"error\":\"{error}\"}}");
        std::process::exit(1);
    }
    let channel = session.video_channel().unwrap_or(0);
    if let Err(error) = session.play().await {
        println!("{{\"ok\":false,\"stage\":\"play\",\"error\":\"{error}\"}}");
        std::process::exit(1);
    }

    let mut buffer: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
    if let Some(track) = &track {
        for parameter in [track.vps.as_ref(), track.sps.as_ref(), track.pps.as_ref()]
            .into_iter()
            .flatten()
        {
            hevc::append_annexb(parameter, &mut buffer);
        }
    }

    let mut depacketizer = hevc::Depacketizer::new();
    let mut packets: u64 = 0;
    let mut units: u64 = 0;
    let mut access_units: u64 = 0;
    let mut keyframes: u64 = 0;
    let mut media_bytes: u64 = 0;
    let mut other_channel: u64 = 0;
    let mut other_type: u64 = 0;
    let mut unparseable: u64 = 0;
    let mut sequence_gaps: u64 = 0;
    let mut previous: std::collections::HashMap<u8, u16> = std::collections::HashMap::new();
    let mut timestamps: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut first_frame: Option<Duration> = None;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let (packet_channel, packet) = match tokio::time::timeout(
            remaining.min(Duration::from_secs(5)),
            session.next_packet(),
        )
        .await
        {
            Ok(Ok(frame)) => frame,
            Ok(Err(error)) => {
                println!("{{\"ok\":false,\"stage\":\"read\",\"error\":\"{error}\"}}");
                break;
            }
            Err(_) => continue,
        };
        if packet_channel != channel {
            other_channel += 1;
            continue;
        }
        let Some(parsed) = rtp::parse(&packet) else {
            unparseable += 1;
            continue;
        };
        if let Some(previous) = previous.insert(packet_channel, parsed.sequence) {
            if parsed.sequence != previous.wrapping_add(1) {
                sequence_gaps += 1;
            }
        }
        if parsed.payload_type
            != track
                .as_ref()
                .map(|t| t.payload_type)
                .unwrap_or(parsed.payload_type)
        {
            other_type += 1;
            continue;
        }
        packets += 1;
        timestamps.insert(parsed.timestamp);
        media_bytes += parsed.payload.len() as u64;
        for nal in depacketizer.push(&parsed) {
            units += 1;
            if hevc::is_keyframe(&nal) {
                keyframes += 1;
            }
            if first_frame.is_none() && hevc::is_vcl(&nal) {
                first_frame = Some(started.elapsed());
            }
            hevc::append_annexb(&nal, &mut buffer);
        }
        if parsed.marker {
            access_units += 1;
        }
    }

    let _ = session.teardown().await;
    let written = std::fs::write(&out, &buffer).is_ok();
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "{{\"ok\":true,\"channel\":{channel},\"codec\":\"{codec}\",\"framerate\":{framerate},\"packets\":{packets},\"units\":{units},\"access_units\":{access_units},\"keyframes\":{keyframes},\"media_bytes\":{media_bytes},\"timestamps\":{},\"sequence_gaps\":{sequence_gaps},\"other_channel\":{other_channel},\"other_type\":{other_type},\"unparseable\":{unparseable},\"written_bytes\":{},\"dropped\":{},\"elapsed_s\":{elapsed:.2},\"startup_ms\":{},\"written\":{written},\"out\":\"{out}\"}}",
        timestamps.len(),
        buffer.len(),
        depacketizer.dropped(),
        first_frame.map(|d| d.as_millis()).unwrap_or(0)
    );
}
