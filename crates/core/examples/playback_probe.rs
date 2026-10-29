use std::time::{Duration, Instant};

use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::config::Config;
use proview_core::dvrip::playback::{shift, FrameRate, MediaReader};
use proview_core::provider::cloud::CloudProvider;
use proview_core::provider::VendorSession;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    let serial = env("PROVIEW_DEVICE_SERIAL");
    let begin = env("PROVIEW_BEGIN");
    let end = env("PROVIEW_END");
    let dump = env("PROVIEW_DUMP");
    let kind = match env("PROVIEW_FILE_TYPE") {
        value if value.is_empty() => "h264".to_string(),
        value => value,
    };
    let frames_wanted: usize = env("PROVIEW_FRAMES").parse().unwrap_or(48);
    let whole = !env("PROVIEW_WHOLE").is_empty();
    let download = !env("PROVIEW_DOWNLOAD").is_empty();
    let pick = env("PROVIEW_PICK");
    let seek: u32 = env("PROVIEW_SEEK").parse().unwrap_or(0);
    if account.is_empty() || password.is_empty() || serial.is_empty() {
        return Err("set PROVIEW_EMAIL, PROVIEW_PASSWORD and PROVIEW_DEVICE_SERIAL".into());
    }
    if begin.is_empty() || end.is_empty() {
        return Err("set PROVIEW_BEGIN and PROVIEW_END as 'YYYY-MM-DD HH:MM:SS'".into());
    }
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }
    let client = EnvelopeClient::new(identity);
    let started = Instant::now();
    let tokens = client.login_envelope(&account, &password).await?;
    eprintln!("cloud    {:.2}s", started.elapsed().as_secs_f64());
    let vendor = VendorSession {
        account,
        token: Some(tokens.access),
    };

    let provider = CloudProvider::new(&Config::from_env());
    let started = Instant::now();
    let (mut connection, note) = provider.open_playback(&vendor, &serial).await?;
    eprintln!(
        "login    {:.2}s session=0x{:08x} {note}",
        started.elapsed().as_secs_f64(),
        connection.session()
    );

    let started = Instant::now();
    let mut files = connection.files(0, &begin, &end, &kind).await?;
    let total: u64 = files.iter().map(|file| file.bytes).sum();
    eprintln!(
        "files    {:.2}s count={} bytes={total}",
        started.elapsed().as_secs_f64(),
        files.len()
    );
    if let (Some(first), Some(last)) = (files.first(), files.last()) {
        eprintln!("window   {} .. {}", first.begin, last.end);
        eprintln!("first    {} {}", first.name, first.bytes);
    }
    if pick == "largest" {
        files.sort_by_key(|file| std::cmp::Reverse(file.bytes));
    }
    if let Some(chosen) = files.first() {
        eprintln!("chosen   {} {} bytes", chosen.name, chosen.bytes);
    }
    let mut file = match if pick == "largest" {
        files.first()
    } else {
        files.last()
    } {
        Some(file) => file.clone(),
        None => return Ok(()),
    };
    if seek > 0 {
        match shift(&file.begin, seek) {
            Some(begin) => file.begin = begin,
            None => return Err("the file begin is not a device timestamp".into()),
        }
        eprintln!("seek     +{seek}s -> {}", file.begin);
    }

    let started = Instant::now();
    let claim = connection.claim(&file, 0).await?;
    eprintln!(
        "claim    {:.2}s ret={claim}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    if download {
        connection.download_start(&file, 0).await?;
        eprintln!("download sent in {:.2}s", started.elapsed().as_secs_f64());
    } else {
        connection.start(&file, 0).await?;
        eprintln!("start    sent in {:.2}s", started.elapsed().as_secs_f64());
    }

    let mut reader = MediaReader::new();
    let mut rate = FrameRate::default();
    let mut records = 0usize;
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut deltas_since_key = 0usize;
    let mut key_stamps: Vec<u32> = Vec::new();
    let mut sink = Vec::new();
    let mut frames = 0usize;
    let mut bytes = 0usize;
    let started = Instant::now();
    let mut first_byte = None;
    let mut last_frame = 0.0f64;
    let limit: f64 = env("PROVIEW_LIMIT").parse().unwrap_or(0.0);
    while frames < frames_wanted || whole {
        if limit > 0.0 && started.elapsed().as_secs_f64() > limit {
            eprintln!(
                "limit    stopping after {:.2}s",
                started.elapsed().as_secs_f64()
            );
            break;
        }
        match connection.frame(Duration::from_secs(8)).await? {
            Some(media) => {
                last_frame = started.elapsed().as_secs_f64();
                if first_byte.is_none() {
                    first_byte = Some(started.elapsed());
                    eprintln!(
                        "first    frame after {:.3}s",
                        started.elapsed().as_secs_f64()
                    );
                }
                frames += 1;
                bytes += media.payload.len();
                reader.push(&media.payload);
                while let Some(record) = reader.next_record() {
                    records += 1;
                    if record.kind == proview_core::dvrip::playback::RecordKind::Key {
                        if let Some(seconds) = record
                            .stamp
                            .and_then(proview_core::dvrip::playback::stamp_seconds)
                        {
                            eprintln!(
                                "gop      key {seconds} after {deltas_since_key} deltas frames={} keys={} settled={} fps={:.3}",
                                rate.frames(),
                                rate.keys(),
                                rate.settled(),
                                rate.per_second
                            );
                        }
                        deltas_since_key = 0;
                    } else if record.kind == proview_core::dvrip::playback::RecordKind::Delta {
                        deltas_since_key += 1;
                    }
                    *kinds.entry(format!("{:?}", record.kind)).or_insert(0) += 1;
                    rate.observe(&record);
                    if let Some(stamp) = record.stamp {
                        if whole || key_stamps.len() < 4 {
                            key_stamps.push(stamp);
                        }
                    }
                    if record.kind == proview_core::dvrip::playback::RecordKind::Key
                        || record.kind == proview_core::dvrip::playback::RecordKind::Delta
                    {
                        sink.extend_from_slice(&record.bytes);
                    }
                }
                if frames <= 3 || frames % 16 == 0 {
                    let hex: String = media
                        .payload
                        .iter()
                        .take(32)
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    eprintln!(
                        "frame{frames:<3} {:.2}s id=0x{:04x} ch={} len={} {hex}",
                        started.elapsed().as_secs_f64(),
                        media.id,
                        media.channel,
                        media.payload.len()
                    );
                }
            }
            None => {
                eprintln!("frame{frames:<3} nothing inside 8s");
                break;
            }
        }
    }
    eprintln!(
        "stream   chunks={frames} bytes={bytes} first_frame={:?} last_frame={last_frame:.2}s elapsed={:.2}s",
        first_byte,
        started.elapsed().as_secs_f64()
    );
    let throughput = if last_frame > 0.0 {
        file.bytes as f64 / last_frame
    } else {
        0.0
    };
    eprintln!(
        "wire     bytes={bytes} listed={} match={} rate={throughput:.0}B/s",
        file.bytes,
        bytes == file.bytes as usize
    );
    eprintln!("kinds    {kinds:?}");
    eprintln!(
        "records  {records} skipped={} fps={:.3} settled={} span={} keys={} key_stamps={:?}",
        reader.skipped(),
        rate.per_second,
        rate.settled(),
        rate.span_seconds(),
        rate.keys(),
        key_stamps
    );
    if !dump.is_empty() {
        std::fs::write(&dump, &sink)?;
        eprintln!("dump     {dump} {} bytes", sink.len());
    }
    if whole {
        eprintln!(
            "whole    bytes={bytes} wall={:.2}s keys={} first_key={} last_key={}",
            started.elapsed().as_secs_f64(),
            key_stamps.len(),
            key_stamps
                .first()
                .map(|stamp| proview_core::dvrip::playback::stamp_seconds(*stamp).unwrap_or(0))
                .unwrap_or(0),
            key_stamps
                .last()
                .map(|stamp| proview_core::dvrip::playback::stamp_seconds(*stamp).unwrap_or(0))
                .unwrap_or(0)
        );
    }
    if download {
        let _ = connection.download_stop(&file, 0).await;
    } else {
        let _ = connection.stop(&file, 0).await;
    }
    let _ = connection.logout().await;
    Ok(())
}
