use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use proview_core::live::{LiveHub, LiveMode, StreamKey};
use proview_core::media::{h264, nal};
use proview_core::source::{pump, RtspTarget};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

const SDP: &str = "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=RTSP Server\r\na=control:*\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=framerate:25\r\na=control:trackID=1\r\n";

async fn read_request(socket: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await.expect("rtsp request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn access_units() -> Vec<Vec<Vec<u8>>> {
    let clip = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/dual-lens/dual-lens-stacked-640x720.h264"
    ))
    .expect("fixture clip");
    let mut units: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut sets: Vec<Vec<u8>> = Vec::new();
    for unit in nal::split_annexb(&clip) {
        if !h264::is_vcl(unit.data) {
            sets.push(unit.data.to_vec());
            continue;
        }
        if h264::is_picture_start(unit.data) {
            units.push(std::mem::take(&mut sets));
        }
        match units.last_mut() {
            Some(current) => current.push(unit.data.to_vec()),
            None => units.push(vec![unit.data.to_vec()]),
        }
    }
    units
}

async fn serve(
    listener: TcpListener,
    units: Vec<Vec<Vec<u8>>>,
    pause_after: usize,
    sent: Arc<Notify>,
    resume: Arc<Notify>,
) {
    let (mut socket, _) = listener.accept().await.expect("accept");
    let describe = read_request(&mut socket).await;
    assert!(describe.starts_with("DESCRIBE "), "{describe}");
    let response = format!(
        "RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{SDP}",
        SDP.len()
    );
    socket.write_all(response.as_bytes()).await.expect("sdp");
    let setup = read_request(&mut socket).await;
    assert!(setup.starts_with("SETUP "), "{setup}");
    socket
        .write_all(
            b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nSession: 1234;timeout=60\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1;mode=PLAY\r\n\r\n",
        )
        .await
        .expect("setup");
    let play = read_request(&mut socket).await;
    assert!(play.starts_with("PLAY "), "{play}");
    socket
        .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: 1234\r\n\r\n")
        .await
        .expect("play");
    let mut sequence = 0u16;
    for (index, unit) in units.iter().enumerate() {
        if index == pause_after {
            sent.notify_one();
            resume.notified().await;
        }
        let timestamp = (index as u32) * 3_600;
        for (position, payload) in unit.iter().enumerate() {
            let marker = position + 1 == unit.len();
            let mut packet = vec![0x80, if marker { 0x80 } else { 0 } | 96];
            packet.extend_from_slice(&sequence.to_be_bytes());
            packet.extend_from_slice(&timestamp.to_be_bytes());
            packet.extend_from_slice(&0x1122_3344u32.to_be_bytes());
            packet.extend_from_slice(payload);
            sequence = sequence.wrapping_add(1);
            let mut frame = vec![b'$', 0];
            frame.extend_from_slice(&(packet.len() as u16).to_be_bytes());
            frame.extend_from_slice(&packet);
            if socket.write_all(&frame).await.is_err() {
                return;
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), read_request(&mut socket)).await;
}

fn opens_on_idr(fragment: &[u8]) -> bool {
    let mdat = fragment
        .windows(4)
        .position(|window| window == b"mdat")
        .expect("mdat");
    let mut offset = mdat + 4;
    while offset + 4 <= fragment.len() {
        let length = u32::from_be_bytes([
            fragment[offset],
            fragment[offset + 1],
            fragment[offset + 2],
            fragment[offset + 3],
        ]) as usize;
        if length == 0 || offset + 4 + length > fragment.len() {
            return false;
        }
        let nal = &fragment[offset + 4..offset + 4 + length];
        if h264::is_vcl(nal) {
            return h264::nal_type(nal) == Some(5);
        }
        offset += 4 + length;
    }
    false
}

fn sequence_number(fragment: &[u8]) -> u32 {
    let mfhd = fragment
        .windows(4)
        .position(|window| window == b"mfhd")
        .expect("mfhd");
    u32::from_be_bytes([
        fragment[mfhd + 8],
        fragment[mfhd + 9],
        fragment[mfhd + 10],
        fragment[mfhd + 11],
    ])
}

async fn subscribe_when_fragments_exist(
    hub: &Arc<LiveHub>,
    key: &StreamKey,
) -> proview_core::live::Subscription {
    for _ in 0..500 {
        let candidate = hub.subscribe(key.clone());
        if !candidate.replay.is_empty() {
            return candidate;
        }
        hub.release(key);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no fragments were published");
}

#[tokio::test]
async fn a_late_viewer_gets_the_gop_and_then_an_unbroken_stream() {
    let units = access_units();
    assert!(units.len() >= 14, "{} access units", units.len());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
    let addr = listener.local_addr().expect("addr");
    let sent = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let server = tokio::spawn(serve(listener, units, 12, sent.clone(), resume.clone()));

    let hub = Arc::new(LiveHub::default());
    let key = StreamKey::new("cam-0001", 1, false, LiveMode::Transcode);
    let stop = Arc::new(AtomicBool::new(false));
    let target = RtspTarget::from_host_port(&addr.ip().to_string(), addr.port())
        .with_credentials("admin", "secret");
    let running = tokio::spawn(pump(hub.clone(), key.clone(), target, stop.clone()));

    tokio::time::timeout(Duration::from_secs(10), sent.notified())
        .await
        .expect("the camera reached the pause");
    let mut subscription = tokio::time::timeout(
        Duration::from_secs(10),
        subscribe_when_fragments_exist(&hub, &key),
    )
    .await
    .expect("a fragment reaches the hub");
    let replayed = subscription.replay.clone();
    assert!(
        subscription.init.lock().expect("init slot").is_some(),
        "the init segment must be published before the first fragment"
    );
    assert!(replayed[0].keyframe, "the replay must open on a keyframe");
    assert!(
        opens_on_idr(&replayed[0].bytes),
        "the replayed fragment must open on an IDR access unit"
    );
    let last_replayed = sequence_number(&replayed.last().expect("fragment").bytes);

    resume.notify_one();
    let next = tokio::time::timeout(Duration::from_secs(10), subscription.receiver.recv())
        .await
        .expect("the live stream continues")
        .expect("a fragment");
    assert_eq!(
        sequence_number(&next.bytes),
        last_replayed + 1,
        "the live stream continues exactly after the replayed fragments"
    );
    stop.store(true, Ordering::Relaxed);
    running.abort();
    let _ = tokio::time::timeout(Duration::from_secs(10), server).await;
}

#[tokio::test]
async fn a_viewer_that_falls_behind_resumes_on_a_keyframe_fragment() {
    let units = access_units();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
    let addr = listener.local_addr().expect("addr");
    let sent = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let server = tokio::spawn(serve(listener, units, 0, sent.clone(), resume.clone()));

    let hub = Arc::new(LiveHub::new(1));
    let key = StreamKey::new("cam-0002", 1, false, LiveMode::Transcode);
    let mut subscription = hub.subscribe(key.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let target = RtspTarget::from_host_port(&addr.ip().to_string(), addr.port())
        .with_credentials("admin", "secret");
    let running = tokio::spawn(pump(hub.clone(), key.clone(), target, stop.clone()));
    resume.notify_one();
    tokio::time::timeout(Duration::from_secs(20), running)
        .await
        .expect("the source settles once the camera hangs up")
        .expect("pump task");

    let mut lagged = false;
    let mut resumed_on_keyframe = false;
    let observed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match subscription.receiver.recv().await {
                Ok(packet) => {
                    if lagged && packet.keyframe {
                        resumed_on_keyframe = true;
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    lagged = true;
                }
                Err(_) => break,
            }
        }
    })
    .await;
    observed.expect("the receiver settles");
    assert!(lagged, "the small ring must report a lagged viewer");
    assert!(
        resumed_on_keyframe,
        "a viewer that fell behind must be able to resume on a keyframe fragment"
    );
    stop.store(true, Ordering::Relaxed);
    server.await.expect("camera task");
}
