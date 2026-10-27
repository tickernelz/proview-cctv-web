use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use proview_core::dvrip::client::Connection;
use proview_core::dvrip::frame::Header;
use proview_core::dvrip::settings::{EncodePatch, NetworkPatch, Stream};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Fake {
    blocks: Mutex<Vec<(String, Value)>>,
    writes: Mutex<Vec<(String, Value)>>,
    honour_writes: bool,
}

impl Fake {
    fn new(honour_writes: bool) -> Arc<Self> {
        Arc::new(Self {
            blocks: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            honour_writes,
        })
    }

    fn seed(&self, name: &str, value: Value) {
        self.blocks.lock().unwrap().push((name.to_string(), value));
    }

    fn block(&self, name: &str) -> Option<Value> {
        self.blocks
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    fn replies(&self) -> usize {
        self.writes.lock().unwrap().len()
    }
}

async fn read_frame(socket: &mut TcpStream) -> Option<(Header, Value)> {
    let mut header_bytes = [0u8; 20];
    socket.read_exact(&mut header_bytes).await.ok()?;
    let header = Header::decode(&header_bytes).ok()?;
    let mut body = vec![0u8; header.length as usize];
    if header.length > 0 {
        socket.read_exact(&mut body).await.ok()?;
    }
    let end = body
        .iter()
        .position(|byte| *byte == 0x0a)
        .unwrap_or(body.len());
    let value = serde_json::from_slice::<Value>(&body[..end]).ok()?;
    Some((header, value))
}

async fn write_frame(socket: &mut TcpStream, header: &Header, body: &Value) {
    let mut payload = serde_json::to_vec(body).unwrap();
    payload.push(b'\n');
    payload.push(0);
    let reply = Header {
        session: 0,
        sequence: header.sequence,
        channel: 0,
        end: true,
        id: header.id,
        length: payload.len() as u32,
    };
    socket.write_all(&reply.encode()).await.unwrap();
    socket.write_all(&payload).await.unwrap();
    socket.flush().await.unwrap();
}

async fn device(fake: Arc<Fake>) -> (Connection, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        while let Some((header, request)) = read_frame(&mut socket).await {
            let name = request
                .get("Name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match header.id {
                proview_core::dvrip::msg::CONFIG_GET | proview_core::dvrip::msg::SYSTEM_INFO => {
                    let block = fake.block(&name).unwrap_or(Value::Null);
                    let mut reply = json!({ "Ret": if block.is_null() { 607 } else { 100 }, "SessionID": "0x00000000" });
                    if !block.is_null() {
                        reply[name.clone()] = block;
                    }
                    write_frame(&mut socket, &header, &reply).await;
                }
                proview_core::dvrip::msg::CONFIG_SET => {
                    let value = request.get(name.clone()).cloned().unwrap_or(Value::Null);
                    fake.writes
                        .lock()
                        .unwrap()
                        .push((name.clone(), value.clone()));
                    if fake.honour_writes {
                        let mut blocks = fake.blocks.lock().unwrap();
                        blocks.retain(|(key, _)| key != &name);
                        blocks.push((name.clone(), value));
                    }
                    write_frame(
                        &mut socket,
                        &header,
                        &json!({ "Ret": 100, "SessionID": "0x00000000" }),
                    )
                    .await;
                }
                _ => write_frame(
                    &mut socket,
                    &header,
                    &json!({ "Ret": 100, "SessionID": "0x00000000" }),
                )
                .await,
            }
        }
    });
    let stream = TcpStream::connect(address).await.unwrap();
    (Connection::from_stream(stream), handle)
}

fn backups() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "zha4013-settings-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn encode_block(fps: i64, bitrate: i64) -> Value {
    json!({
        "MainFormat": {
            "VideoEnable": true,
            "Video": {
                "BitRate": bitrate,
                "BitRateControl": "CBR",
                "Compression": "H.264",
                "FPS": fps,
                "GOP": 36,
                "Quality": 3,
                "Resolution": "3072*2048"
            }
        },
        "ExtraFormat": {
            "Video": {
                "BitRate": 512,
                "Compression": "H.264",
                "FPS": 12,
                "GOP": 24,
                "Resolution": "640*360"
            }
        }
    })
}

fn network_block() -> Value {
    json!({
        "HostIP": "192.168.0.100",
        "SubMask": "255.255.255.0",
        "GateWay": "192.168.0.1",
        "MAC": "3C:EF:8C:11:22:33",
        "UserName": "admin",
        "Password": "device-password",
        "TCPPort": 34567
    })
}

#[tokio::test]
async fn a_write_that_reads_back_keeps_the_new_value_and_saves_the_old_one() {
    let fake = Fake::new(true);
    fake.seed("Simplify.Encode", encode_block(12, 1536));
    let (mut connection, handle) = device(fake.clone()).await;
    let dir = backups();
    let report = connection
        .write_encode(
            Stream::Main,
            &EncodePatch {
                fps: Some(15),
                bitrate: Some(2048),
                ..EncodePatch::default()
            },
            &dir,
        )
        .await
        .unwrap();
    assert!(!report.reverted);
    assert!(report.matched());
    let fps = report
        .fields
        .iter()
        .find(|field| field.field == "MainFormat.Video.FPS")
        .unwrap();
    assert_eq!(fps.before, json!(12));
    assert_eq!(fps.requested, json!(15));
    assert_eq!(fps.after, json!(15));
    assert_eq!(connection.encode_state().await.unwrap().main.bitrate, Some(2048));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&report.backup).unwrap()).unwrap();
    assert_eq!(saved["block"], "Simplify.Encode");
    assert_eq!(saved["value"]["MainFormat"]["Video"]["FPS"], json!(12));
    assert_eq!(fake.replies(), 1);
    handle.abort();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_write_the_device_does_not_hold_is_put_back_from_the_saved_copy() {
    let fake = Fake::new(false);
    fake.seed("Simplify.Encode", encode_block(12, 1536));
    let (mut connection, handle) = device(fake.clone()).await;
    let dir = backups();
    let report = connection
        .write_encode(
            Stream::Main,
            &EncodePatch {
                fps: Some(15),
                ..EncodePatch::default()
            },
            &dir,
        )
        .await
        .unwrap();
    assert!(report.reverted);
    assert!(!report.matched());
    assert_eq!(report.fields[0].before, json!(12));
    assert_eq!(report.fields[0].requested, json!(15));
    assert_eq!(report.fields[0].after, json!(12));
    assert!(report.restored.iter().all(|field| field.matches));
    let writes = fake.writes.lock().unwrap().clone();
    assert_eq!(writes.len(), 2, "one write and one revert");
    assert_eq!(writes[1].1["MainFormat"]["Video"]["FPS"], json!(12));
    assert_eq!(connection.encode_state().await.unwrap().main.fps, Some(12));
    handle.abort();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_network_write_touches_only_the_named_field_and_never_echoes_credentials() {
    let fake = Fake::new(true);
    fake.seed("NetWork.NetCommon", network_block());
    let (mut connection, handle) = device(fake.clone()).await;
    let dir = backups();
    let reports = connection
        .write_network(
            &NetworkPatch {
                gateway: Some("192.168.0.2".to_string()),
                ..NetworkPatch::default()
            },
            &dir,
        )
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].reverted);
    assert_eq!(reports[0].fields.len(), 1);
    assert_eq!(reports[0].fields[0].field, "GateWay");
    assert_eq!(reports[0].fields[0].before, json!("192.168.0.1"));
    assert_eq!(reports[0].fields[0].after, json!("192.168.0.2"));
    let written = &fake.writes.lock().unwrap()[0].1;
    assert_eq!(written["HostIP"], json!("192.168.0.100"));
    assert_eq!(written["GateWay"], json!("192.168.0.2"));
    assert_eq!(reports[0].written["Password"], json!("<redacted>"));
    assert_eq!(reports[0].written["UserName"], json!("<redacted>"));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&reports[0].backup).unwrap()).unwrap();
    assert_eq!(saved["value"]["GateWay"], json!("192.168.0.1"));
    handle.abort();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn storage_reads_the_card_the_overwrite_rule_and_the_record_mode() {
    let fake = Fake::new(true);
    fake.seed(
        "StorageInfo",
        json!({
            "PlysicalNo": 0,
            "PartNumber": 1,
            "Partition": [{
                "IsCurrent": true,
                "Status": 1,
                "TotalSpace": "30528MB",
                "RemainSpace": "29303MB",
                "OldStartTime": "2026-10-20 11:02:07",
                "OldEndTime": "2026-10-27 13:55:41"
            }]
        }),
    );
    fake.seed("General.General", json!({ "OverWrite": "OverWrite", "MachineName": "cam" }));
    fake.seed("Record", json!([{ "RecordMode": "ConfigRecord" }]));
    let (mut connection, handle) = device(fake.clone()).await;
    let state = connection.storage_state().await.unwrap();
    assert_eq!(state.partitions.len(), 1);
    assert_eq!(state.partitions[0].remain_space.as_deref(), Some("29303MB"));
    assert_eq!(state.partitions[0].current, Some(true));
    assert_eq!(state.overwrite.as_deref(), Some("OverWrite"));
    assert_eq!(state.record_mode.as_deref(), Some("ConfigRecord"));
    handle.abort();
}

#[tokio::test]
async fn a_record_mode_write_lands_on_the_first_record_entry() {
    let fake = Fake::new(true);
    fake.seed(
        "Record",
        json!([{ "RecordMode": "ConfigRecord", "PreRecord": 0 }]),
    );
    let (mut connection, handle) = device(fake.clone()).await;
    let dir = backups();
    let report = connection
        .write_record_mode(proview_core::dvrip::control::RecordMode::Manual, &dir)
        .await
        .unwrap();
    assert!(!report.reverted);
    assert_eq!(report.fields[0].field, "0.RecordMode");
    assert_eq!(report.fields[0].after, json!("ManualRecord"));
    assert_eq!(connection.storage_state().await.unwrap().record_mode.as_deref(), Some("ManualRecord"));
    handle.abort();
    std::fs::remove_dir_all(&dir).ok();
}
