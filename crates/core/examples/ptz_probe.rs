use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient};
use proview_core::dvrip::client::Connection;
use proview_core::dvrip::msg;
use proview_core::provider::{p2p, rps};
use serde_json::{json, Value};

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn number(key: &str, fallback: i64) -> i64 {
    let raw = env(key);
    if raw.is_empty() {
        fallback
    } else {
        raw.parse().unwrap_or(fallback)
    }
}

fn out_dir() -> PathBuf {
    let raw = env("PROVIEW_OUT");
    let dir = if raw.is_empty() {
        PathBuf::from("ptz-probe")
    } else {
        PathBuf::from(raw)
    };
    std::fs::create_dir_all(&dir).expect("create the artifact directory");
    dir
}

fn write_json(dir: &Path, name: &str, value: &Value) {
    let path = dir.join(name);
    if let Err(error) = std::fs::write(&path, serde_json::to_vec_pretty(value).expect("json")) {
        eprintln!("artifact {} failed: {error}", path.display());
        return;
    }
    eprintln!("artifact {}", path.display());
}

#[derive(Debug, Clone)]
struct PtzCommand {
    command: String,
    channel: i64,
    step: i64,
    tour: i64,
    preset: i64,
    preset_index: i64,
    times: i64,
}

impl PtzCommand {
    fn new(command: &str) -> Self {
        Self {
            command: command.to_string(),
            channel: 0,
            step: 3,
            tour: 0,
            preset: 0,
            preset_index: 0,
            times: 1,
        }
    }

    fn channel(mut self, channel: i64) -> Self {
        self.channel = channel;
        self
    }

    fn tour(mut self, tour: i64) -> Self {
        self.tour = tour;
        self
    }

    fn preset(mut self, preset: i64) -> Self {
        self.preset = preset;
        self
    }

    fn preset_index(mut self, index: i64) -> Self {
        self.preset_index = index;
        self
    }
}

async fn open_connection() -> Result<Connection, Box<dyn std::error::Error>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    let serial = env("PROVIEW_DEVICE_SERIAL");
    let device_user = env("PROVIEW_DEVICE_USER");
    let device_pass = env("PROVIEW_DEVICE_PASS");
    if account.is_empty() || password.is_empty() || serial.is_empty() {
        return Err("set PROVIEW_EMAIL, PROVIEW_PASSWORD and PROVIEW_DEVICE_SERIAL".into());
    }
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }
    let client = EnvelopeClient::new(identity);
    let tokens = client.login_envelope(&account, &password).await?;
    let devices = client.device_list(&tokens.access).await?;
    let device = devices
        .iter()
        .find(|device| device.uuid == serial)
        .ok_or("the serial is not on this account")?;
    let rps_id = device
        .extra
        .get("rps")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let status = StatusClient::new("pub-status.secu100.net")
        .query(&serial, &auth_code_of(device))
        .await?;
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
    let agent = rps::AccessClient::new()
        .assign(&access_host, access_port, &body)
        .await?;
    let frame = p2p::handshake_frame(&rps_id, &token, &serial, &session);
    let tunnel = rps::open(&agent, &frame).await?;
    let mut connection = tunnel.into_connection();
    connection.login(&device_user, &device_pass).await?;
    Ok(connection)
}

async fn ptz(connection: &mut Connection, command: PtzCommand) -> Value {
    let body = json!({
        "Name": "OPPTZControl",
        "SessionID": format!("0x{:08X}", connection.session()),
        "OPPTZControl": {
            "Command": command.command,
            "Parameter": {
                "AUX": { "Number": 0, "Status": "On" },
                "Channel": command.channel,
                "MenuOpts": "Enter",
                "Pattern": "SetBegin",
                "Preset": command.preset,
                "PresetIndex": command.preset_index,
                "Step": command.step,
                "Tour": command.tour,
                "TourTimes": command.times,
            },
        },
    });
    match connection.request(msg::PTZ_CONTROL, body).await {
        Ok(reply) => reply,
        Err(error) => json!({ "error": error.to_string() }),
    }
}

async fn tours(connection: &mut Connection) -> Value {
    match connection.config_get("Uart").await {
        Ok(reply) => reply
            .get("Uart")
            .and_then(|uart| uart.get("PTZTour"))
            .cloned()
            .unwrap_or(Value::Null),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

async fn snap(connection: &mut Connection, dir: &Path, name: &str, log: &mut Vec<Value>) {
    let started = Instant::now();
    match connection.snapshot(0).await {
        Ok(bytes) => {
            let path = dir.join(name);
            if let Err(error) = std::fs::write(&path, &bytes) {
                log.push(json!({ "file": name, "error": error.to_string() }));
                return;
            }
            eprintln!(
                "snap {name} {} bytes {} ms",
                bytes.len(),
                started.elapsed().as_millis()
            );
            log.push(json!({ "file": name, "bytes": bytes.len(), "t_ms": started.elapsed().as_millis() as u64 }));
        }
        Err(error) => log.push(json!({ "file": name, "error": error.to_string() })),
    }
}

async fn move_and_settle(connection: &mut Connection, channel: i64, hold_ms: u64) -> Value {
    let start = ptz(
        connection,
        PtzCommand::new("DirectionRight")
            .channel(channel)
            .preset(65535),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(hold_ms)).await;
    let stop = ptz(
        connection,
        PtzCommand::new("DirectionRight")
            .channel(channel)
            .preset(-1),
    )
    .await;
    json!({ "channel": channel, "start": start, "stop": stop })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = out_dir();
    let phase = {
        let raw = env("PROVIEW_PHASE");
        if raw.is_empty() {
            "lens".to_string()
        } else {
            raw
        }
    };
    let mut connection = open_connection().await?;
    eprintln!("dvrip session={} phase={phase}", connection.session());
    let mut snaps: Vec<Value> = Vec::new();
    let mut report = json!({ "phase": phase, "session": connection.session() });

    if phase == "lens" {
        let hold = number("PROVIEW_HOLD_MS", 400).max(100) as u64;
        snap(&mut connection, &dir, "00-before.jpg", &mut snaps).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        snap(&mut connection, &dir, "01-before-second.jpg", &mut snaps).await;
        let mut moves = Vec::new();
        moves.push(move_and_settle(&mut connection, 0, hold).await);
        tokio::time::sleep(Duration::from_millis(900)).await;
        snap(&mut connection, &dir, "02-after-channel0.jpg", &mut snaps).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        snap(
            &mut connection,
            &dir,
            "03-after-channel0-second.jpg",
            &mut snaps,
        )
        .await;
        moves.push(move_and_settle(&mut connection, 1, hold).await);
        tokio::time::sleep(Duration::from_millis(900)).await;
        snap(&mut connection, &dir, "04-after-channel1.jpg", &mut snaps).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        snap(
            &mut connection,
            &dir,
            "05-after-channel1-second.jpg",
            &mut snaps,
        )
        .await;
        report["moves"] = Value::Array(moves);
        report["hold_ms"] = json!(hold);
        report["snaps"] = Value::Array(snaps);
        write_json(&dir, "lens.json", &report);
        let _ = connection.logout().await;
        return Ok(());
    }

    if phase == "tour" {
        let tour = number("PROVIEW_TOUR", 1);
        let preset_a = number("PROVIEW_PRESET_A", 1);
        let preset_b = number("PROVIEW_PRESET_B", 2);
        let mut steps: Vec<Value> = Vec::new();
        report["tours_before"] = tours(&mut connection).await;
        for (index, preset) in [(0i64, preset_a), (1i64, preset_b)] {
            let reply = ptz(
                &mut connection,
                PtzCommand::new("AddTour")
                    .tour(tour)
                    .preset(preset)
                    .preset_index(index),
            )
            .await;
            steps.push(json!({ "step": "AddTour", "tour": tour, "preset": preset, "preset_index": index, "reply": reply }));
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        report["tours_after_add"] = tours(&mut connection).await;

        snap(&mut connection, &dir, "10-before-tour.jpg", &mut snaps).await;
        let started = Instant::now();
        let start = ptz(&mut connection, PtzCommand::new("StartTour").tour(tour)).await;
        steps.push(json!({ "step": "StartTour", "tour": tour, "reply": start }));
        for index in 0..10 {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let name = format!("11-running-{index}.jpg");
            snap(&mut connection, &dir, &name, &mut snaps).await;
        }
        let stop = ptz(&mut connection, PtzCommand::new("StopTour").tour(tour)).await;
        steps.push(json!({
            "step": "StopTour",
            "tour": tour,
            "reply": stop,
            "ran_ms": started.elapsed().as_millis() as u64,
        }));
        for index in 0..4 {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let name = format!("12-stopped-{index}.jpg");
            snap(&mut connection, &dir, &name, &mut snaps).await;
        }
        report["tours_after_stop"] = tours(&mut connection).await;

        if env("PROVIEW_KEEP_TOUR").is_empty() {
            let cleared = ptz(&mut connection, PtzCommand::new("ClearTour").tour(tour)).await;
            steps.push(json!({ "step": "ClearTour", "tour": tour, "reply": cleared }));
            tokio::time::sleep(Duration::from_millis(400)).await;
            report["tours_after_clear"] = tours(&mut connection).await;
        }
        report["steps"] = Value::Array(steps);
        report["snaps"] = Value::Array(snaps);
        write_json(&dir, "tour.json", &report);
        let _ = connection.logout().await;
        return Ok(());
    }

    if phase == "control" {
        use proview_core::provider::Provider;
        let serial = env("PROVIEW_DEVICE_SERIAL");
        let cfg = proview_core::config::Config::from_env();
        let provider = proview_core::provider::cloud::CloudProvider::new(&cfg);
        let account = provider
            .login(&env("PROVIEW_EMAIL"), &env("PROVIEW_PASSWORD"))
            .await?;
        let started = Instant::now();
        let mut connection = provider.open_control(&account, &serial).await?;
        let mut timings = vec![json!({
            "read": "open_control",
            "ms": started.elapsed().as_millis() as u64,
            "session": connection.session(),
        })];
        for name in ["SystemFunction", "Ptz", "Uart"] {
            let started = Instant::now();
            let outcome = tokio::time::timeout(Duration::from_secs(8), async {
                match name {
                    "SystemFunction" => connection
                        .abilities()
                        .await
                        .map(|value| json!({ "keys": value.as_object().map(|map| map.len()).unwrap_or(0) })),
                    other => connection.config_get(other).await.map(|value| {
                        json!({ "bytes": serde_json::to_string(&value).map(|text| text.len()).unwrap_or(0) })
                    }),
                }
            })
            .await;
            let entry = match outcome {
                Ok(Ok(value)) => {
                    json!({ "read": name, "ms": started.elapsed().as_millis() as u64, "value": value })
                }
                Ok(Err(error)) => {
                    json!({ "read": name, "ms": started.elapsed().as_millis() as u64, "error": error.to_string() })
                }
                Err(_) => {
                    json!({ "read": name, "ms": started.elapsed().as_millis() as u64, "error": "timeout after 8 s" })
                }
            };
            eprintln!("{}", entry);
            timings.push(entry);
        }
        report["reads"] = Value::Array(timings);
        write_json(&dir, "control.json", &report);
        let _ = connection.logout().await;
        return Ok(());
    }

    if phase == "token-read" {
        use proview_core::provider::Provider;
        let serial = env("PROVIEW_DEVICE_SERIAL");
        let cfg = proview_core::config::Config::from_env();
        let provider = proview_core::provider::cloud::CloudProvider::new(&cfg);
        let account = provider
            .login(&env("PROVIEW_EMAIL"), &env("PROVIEW_PASSWORD"))
            .await?;
        let started = Instant::now();
        let mut connection = provider.open_media(&account, &serial, 1, false).await?;
        let opened = started.elapsed().as_millis() as u64;
        let mut timings = vec![json!({
            "read": "open_media",
            "ms": opened,
            "session": connection.session(),
        })];
        for name in ["Ptz", "Uart", "SystemFunction"] {
            let started = Instant::now();
            let outcome = tokio::time::timeout(Duration::from_secs(8), async {
                match name {
                    "SystemFunction" => connection
                        .abilities()
                        .await
                        .map(|value| json!({ "keys": value.as_object().map(|map| map.len()).unwrap_or(0) })),
                    other => connection.config_get(other).await.map(|value| {
                        json!({ "bytes": serde_json::to_string(&value).map(|text| text.len()).unwrap_or(0) })
                    }),
                }
            })
            .await;
            let entry = match outcome {
                Ok(Ok(value)) => {
                    json!({ "read": name, "ms": started.elapsed().as_millis() as u64, "value": value })
                }
                Ok(Err(error)) => {
                    json!({ "read": name, "ms": started.elapsed().as_millis() as u64, "error": error.to_string() })
                }
                Err(_) => {
                    json!({ "read": name, "ms": started.elapsed().as_millis() as u64, "error": "timeout after 8 s" })
                }
            };
            eprintln!("{}", entry);
            timings.push(entry);
        }
        report["reads"] = Value::Array(timings);
        write_json(&dir, "token-read.json", &report);
        let _ = connection.logout().await;
        return Ok(());
    }

    if phase == "token-ptz" {
        use proview_core::provider::Provider;
        let serial = env("PROVIEW_DEVICE_SERIAL");
        let cfg = proview_core::config::Config::from_env();
        let provider = proview_core::provider::cloud::CloudProvider::new(&cfg);
        let account = provider
            .login(&env("PROVIEW_EMAIL"), &env("PROVIEW_PASSWORD"))
            .await?;
        let mut connection = provider.open_media(&account, &serial, 1, false).await?;
        let mut timings = Vec::new();
        for step in ["stop", "locate"] {
            let started = Instant::now();
            let outcome = tokio::time::timeout(Duration::from_secs(8), async {
                match step {
                    "stop" => connection
                        .ptz_stop(proview_core::dvrip::control::PtzDirection::Right, 3)
                        .await
                        .map(|_| json!({ "ok": true })),
                    _ => connection
                        .ptz_locate()
                        .await
                        .map(|locate| json!({ "locate": locate.is_some() })),
                }
            })
            .await;
            let entry = match outcome {
                Ok(Ok(value)) => {
                    json!({ "step": step, "ms": started.elapsed().as_millis() as u64, "value": value })
                }
                Ok(Err(error)) => {
                    json!({ "step": step, "ms": started.elapsed().as_millis() as u64, "error": error.to_string() })
                }
                Err(_) => {
                    json!({ "step": step, "ms": started.elapsed().as_millis() as u64, "error": "timeout after 8 s" })
                }
            };
            eprintln!("{}", entry);
            timings.push(entry);
        }
        report["steps"] = Value::Array(timings);
        write_json(&dir, "token-ptz.json", &report);
        let _ = connection.logout().await;
        return Ok(());
    }

    if phase == "preset" {
        let preset = number("PROVIEW_PRESET", 1);
        let mut steps: Vec<Value> = Vec::new();
        snap(&mut connection, &dir, "20-before.jpg", &mut snaps).await;
        let reply = ptz(
            &mut connection,
            PtzCommand::new("GotoPreset").preset(preset),
        )
        .await;
        steps.push(json!({ "step": "GotoPreset", "preset": preset, "reply": reply }));
        tokio::time::sleep(Duration::from_millis(2500)).await;
        snap(&mut connection, &dir, "21-goto.jpg", &mut snaps).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        snap(&mut connection, &dir, "22-goto-second.jpg", &mut snaps).await;
        report["steps"] = Value::Array(steps);
        report["snaps"] = Value::Array(snaps);
        write_json(&dir, "preset.json", &report);
        let _ = connection.logout().await;
        return Ok(());
    }

    if phase == "config" {
        let names: Vec<String> = env("PROVIEW_CONFIGS")
            .split(',')
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        let mut configs = serde_json::Map::new();
        for name in names {
            let value = match connection.config_get(&name).await {
                Ok(reply) => reply,
                Err(error) => json!({ "error": error.to_string() }),
            };
            configs.insert(name, value);
        }
        write_json(&dir, "configs.json", &Value::Object(configs));
        let _ = connection.logout().await;
        return Ok(());
    }

    Err(format!("unknown PROVIEW_PHASE {phase}").into())
}
