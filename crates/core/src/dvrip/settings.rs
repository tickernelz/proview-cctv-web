use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use super::client::{require_ok, Connection, DvrError};
use super::msg;

pub const ENCODE_BLOCK: &str = "Simplify.Encode";
pub const NETWORK_BLOCK: &str = "NetWork.NetCommon";
pub const DHCP_BLOCK: &str = "NetWork.NetDHCP";
pub const DNS_BLOCK: &str = "NetWork.NetDNS";
pub const GENERAL_BLOCK: &str = "General.General";
pub const RECORD_BLOCK: &str = "Record";
pub const STORAGE_BLOCK: &str = "StorageInfo";

const REDACTED: &str = "<redacted>";

#[derive(Debug)]
pub enum SettingsError {
    Device(DvrError),
    Missing {
        block: &'static str,
        path: String,
    },
    Invalid {
        field: &'static str,
        detail: String,
    },
    Backup {
        path: String,
        detail: String,
    },
    Slow {
        block: &'static str,
        detail: String,
    },
    WriteFailed {
        block: String,
        backup: String,
        detail: String,
    },
    RevertFailed {
        block: String,
        backup: String,
        detail: String,
    },
}

#[derive(Debug, Clone)]
pub struct WriteWindow {
    pub backup_dir: PathBuf,
    pub timeout: std::time::Duration,
}

impl WriteWindow {
    pub fn new(backup_dir: PathBuf) -> Self {
        Self {
            backup_dir,
            timeout: std::time::Duration::from_secs(20),
        }
    }

    pub fn with_timeout(backup_dir: PathBuf, timeout: std::time::Duration) -> Self {
        Self {
            backup_dir,
            timeout,
        }
    }

    fn dir(&self) -> &Path {
        &self.backup_dir
    }

    fn window(&self) -> std::time::Duration {
        self.timeout
    }
}

impl From<DvrError> for SettingsError {
    fn from(error: DvrError) -> Self {
        SettingsError::Device(error)
    }
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Device(error) => write!(f, "{error}"),
            SettingsError::Missing { block, path } => {
                write!(f, "the device returned no {block} block at {path}")
            }
            SettingsError::Invalid { field, detail } => write!(f, "{field}: {detail}"),
            SettingsError::Backup { path, detail } => {
                write!(f, "could not write the pre-write copy at {path}: {detail}")
            }
            SettingsError::Slow { block, detail } => {
                write!(f, "{block} did not answer inside the write window: {detail}")
            }
            SettingsError::WriteFailed {
                block,
                backup,
                detail,
            } => write!(
                f,
                "{block} failed after the copy was saved ({detail}); the pre-write value was put back, and the copy is at {backup}"
            ),
            SettingsError::RevertFailed {
                block,
                backup,
                detail,
            } => write!(
                f,
                "{block} did not read back after the write and the automatic revert failed ({detail}); the pre-write copy is at {backup}"
            ),
        }
    }
}

impl std::error::Error for SettingsError {}

static NULL_BLOCK: Value = Value::Null;

fn first_entry(block: &Value) -> (&Value, &'static str) {
    match block {
        Value::Array(items) => (items.first().unwrap_or(&NULL_BLOCK), "0."),
        _ => (block, ""),
    }
}

fn number(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_f64().map(|float| float as i64))
            .or_else(|| item.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn boolean(value: &Value, key: &str) -> Option<bool> {
    value
        .get(key)
        .and_then(|item| item.as_bool().or_else(|| item.as_i64().map(|n| n != 0)))
}

fn at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cursor = value;
    for segment in path.split('.') {
        cursor = match cursor {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cursor)
}

fn slot<'a>(value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut cursor = value;
    for segment in path.split('.') {
        cursor = match cursor {
            Value::Object(map) => map.get_mut(segment)?,
            Value::Array(items) => items.get_mut(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cursor)
}

fn same(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() < f64::EPSILON,
            _ => false,
        },
        (Value::String(a), Value::String(b)) => a.trim() == b.trim(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).map(|other| same(value, other)).unwrap_or(false))
        }
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

fn hidden(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("pass")
        || lower.contains("pwd")
        || lower.contains("secret")
        || lower.contains("token")
        || lower == "username"
        || lower == "user"
}

fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let masked = hidden(key) && value.is_string();
                    let value = if masked {
                        Value::String(REDACTED.to_string())
                    } else {
                        redact(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldChange {
    pub field: String,
    pub before: Value,
    pub requested: Value,
    pub after: Value,
    pub matches: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteReport {
    pub block: String,
    pub backup: String,
    pub fields: Vec<FieldChange>,
    pub written: Value,
    pub read_back: Value,
    pub reverted: bool,
    pub restored: Vec<FieldChange>,
}

impl WriteReport {
    pub fn matched(&self) -> bool {
        self.fields.iter().all(|field| field.matches)
    }
}

fn describe(changes: &[FieldChange]) -> String {
    let drift: Vec<String> = changes
        .iter()
        .filter(|change| !change.matches)
        .map(|change| {
            format!(
                "{} reads {} instead of {}",
                change.field, change.after, change.requested
            )
        })
        .collect();
    drift.join("; ")
}

fn compare(
    fields: &[String],
    prior: &Value,
    desired: &Value,
    read_back: &Value,
) -> Vec<FieldChange> {
    fields
        .iter()
        .map(|field| {
            let before = at(prior, field).cloned().unwrap_or(Value::Null);
            let requested = at(desired, field).cloned().unwrap_or(Value::Null);
            let after = at(read_back, field).cloned().unwrap_or(Value::Null);
            FieldChange {
                field: field.clone(),
                matches: same(&requested, &after),
                before,
                requested,
                after,
            }
        })
        .collect()
}

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_millis())
        .unwrap_or(0)
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn private_dir(dir: &Path) -> Result<(), SettingsError> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|error| SettingsError::Backup {
        path: dir.display().to_string(),
        detail: error.to_string(),
    })
}

fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn backup_file(name: &str, value: &Value, dir: &Path) -> Result<PathBuf, SettingsError> {
    private_dir(dir)?;
    let at_ms = stamp();
    let path = dir.join(format!("config-backup-{}-{at_ms}.json", slug(name)));
    let body = json!({
        "block": name,
        "captured_at_ms": at_ms,
        "value": value,
    });
    let text = serde_json::to_string_pretty(&body).map_err(|error| SettingsError::Backup {
        path: path.display().to_string(),
        detail: error.to_string(),
    })?;
    let mut file = private_file(&path).map_err(|error| SettingsError::Backup {
        path: path.display().to_string(),
        detail: error.to_string(),
    })?;
    std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|error| {
        SettingsError::Backup {
            path: path.display().to_string(),
            detail: error.to_string(),
        }
    })?;
    Ok(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Main,
    Extra,
}

impl Stream {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "main" | "hd" | "primary" => Some(Stream::Main),
            "extra" | "sub" | "sd" | "secondary" => Some(Stream::Extra),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Stream::Main => "MainFormat",
            Stream::Extra => "ExtraFormat",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Stream::Main => "main",
            Stream::Extra => "extra",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct VideoFormat {
    pub resolution: Option<String>,
    pub compression: Option<String>,
    pub fps: Option<i64>,
    pub bitrate: Option<i64>,
    pub gop: Option<i64>,
    pub quality: Option<i64>,
    pub bitrate_control: Option<String>,
}

impl VideoFormat {
    fn read(block: &Value, format: &str) -> Self {
        let (entry, _) = first_entry(block);
        let video = at(entry, &format!("{format}.Video"))
            .cloned()
            .unwrap_or(Value::Null);
        Self {
            resolution: text(&video, "Resolution"),
            compression: text(&video, "Compression"),
            fps: number(&video, "FPS"),
            bitrate: number(&video, "BitRate"),
            gop: number(&video, "GOP"),
            quality: number(&video, "Quality"),
            bitrate_control: text(&video, "BitRateControl"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EncodeState {
    pub main: VideoFormat,
    pub extra: VideoFormat,
    pub snap: VideoFormat,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EncodeLimits {
    pub max_bitrate: Option<i64>,
}

impl EncodeLimits {
    pub const APP_MAX_FPS: i64 = 60;
    pub const APP_MAX_GOP: i64 = 250;
    pub const APP_MAX_BITRATE: i64 = 16384;
}

#[derive(Debug, Clone, Default)]
pub struct EncodePatch {
    pub fps: Option<i64>,
    pub bitrate: Option<i64>,
    pub gop: Option<i64>,
    pub resolution: Option<String>,
}

impl EncodePatch {
    fn empty(&self) -> bool {
        self.fps.is_none()
            && self.bitrate.is_none()
            && self.gop.is_none()
            && self.resolution.is_none()
    }

    fn validate(&self, limits: EncodeLimits) -> Result<(), SettingsError> {
        if self.empty() {
            return Err(SettingsError::Invalid {
                field: "encode",
                detail: "name at least one of fps, bitrate, gop or resolution".to_string(),
            });
        }
        if let Some(fps) = self.fps {
            if !(1..=EncodeLimits::APP_MAX_FPS).contains(&fps) {
                return Err(SettingsError::Invalid {
                    field: "fps",
                    detail: format!("{fps} is outside 1..{}", EncodeLimits::APP_MAX_FPS),
                });
            }
        }
        if let Some(bitrate) = self.bitrate {
            let ceiling = limits.max_bitrate.unwrap_or(EncodeLimits::APP_MAX_BITRATE);
            if bitrate < 1 || bitrate > ceiling {
                return Err(SettingsError::Invalid {
                    field: "bitrate",
                    detail: format!("{bitrate} kbps is outside 1..{ceiling}"),
                });
            }
        }
        if let Some(gop) = self.gop {
            if !(1..=EncodeLimits::APP_MAX_GOP).contains(&gop) {
                return Err(SettingsError::Invalid {
                    field: "gop",
                    detail: format!("{gop} is outside 1..{}", EncodeLimits::APP_MAX_GOP),
                });
            }
        }
        if let Some(resolution) = self.resolution.as_deref() {
            let trimmed = resolution.trim();
            if trimmed.is_empty() || trimmed.len() > 32 || trimmed.chars().any(char::is_whitespace)
            {
                return Err(SettingsError::Invalid {
                    field: "resolution",
                    detail: format!("{resolution:?} is not a device resolution token"),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteMode {
    Overwrite,
    Stop,
}

impl OverwriteMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "overwrite" | "over" | "ring" => Some(OverwriteMode::Overwrite),
            "stop" | "stoprecord" | "stop_record" => Some(OverwriteMode::Stop),
            _ => None,
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            OverwriteMode::Overwrite => "OverWrite",
            OverwriteMode::Stop => "StopRecord",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Partition {
    pub index: usize,
    pub current: Option<bool>,
    pub status: Option<i64>,
    pub total_space: Option<String>,
    pub remain_space: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageState {
    pub partitions: Vec<Partition>,
    pub overwrite: Option<String>,
    pub record_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkState {
    pub host_ip: Option<String>,
    pub submask: Option<String>,
    pub gateway: Option<String>,
    pub mac: Option<String>,
    pub dhcp: Option<bool>,
    pub dns: Option<String>,
    pub spare_dns: Option<String>,
    pub block: Value,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkPatch {
    pub dns: Option<String>,
    pub spare_dns: Option<String>,
}

impl NetworkPatch {
    pub fn empty(&self) -> bool {
        self.dns.is_none() && self.spare_dns.is_none()
    }
}

impl Connection {
    pub async fn setting_block(&mut self, name: &'static str) -> Result<Value, SettingsError> {
        let body = json!({
            "Name": name,
            "SessionID": format!("0x{:08X}", self.session()),
        });
        let reply = self.request(msg::CONFIG_GET, body).await?;
        require_ok(&reply)?;
        reply
            .get(name)
            .cloned()
            .filter(|value| !value.is_null())
            .ok_or_else(|| SettingsError::Missing {
                block: name,
                path: name.to_string(),
            })
    }

    pub async fn system_block(&mut self, name: &'static str) -> Result<Value, SettingsError> {
        let body = json!({
            "Name": name,
            "SessionID": format!("0x{:08X}", self.session()),
        });
        let reply = self.request(msg::SYSTEM_INFO, body).await?;
        require_ok(&reply)?;
        reply
            .get(name)
            .cloned()
            .filter(|value| !value.is_null())
            .ok_or_else(|| SettingsError::Missing {
                block: name,
                path: name.to_string(),
            })
    }

    async fn setting_block_within(
        &mut self,
        name: &'static str,
        window: std::time::Duration,
    ) -> Result<Value, SettingsError> {
        match tokio::time::timeout(window, self.setting_block(name)).await {
            Ok(result) => result,
            Err(_) => {
                self.discard_late_replies().await;
                Err(SettingsError::Slow {
                    block: name,
                    detail: "the read before the write never came".to_string(),
                })
            }
        }
    }

    async fn put_block(&mut self, name: &'static str, value: &Value) -> Result<(), SettingsError> {
        let mut body = json!({
            "Name": name,
            "SessionID": format!("0x{:08X}", self.session()),
        });
        body[name] = value.clone();
        let reply = self.request(msg::CONFIG_SET, body).await?;
        require_ok(&reply)?;
        Ok(())
    }

    async fn commit_block(
        &mut self,
        name: &'static str,
        value: &Value,
        window: std::time::Duration,
    ) -> Result<Value, SettingsError> {
        tokio::time::timeout(window, self.put_block(name, value))
            .await
            .map_err(|_| SettingsError::Slow {
                block: name,
                detail: "the write frame was sent and the device never acknowledged it".to_string(),
            })??;
        tokio::time::timeout(window, self.setting_block(name))
            .await
            .map_err(|_| SettingsError::Slow {
                block: name,
                detail: "the read-back after the write never came".to_string(),
            })?
    }

    async fn discard_late_replies(&mut self) {
        for _ in 0..4 {
            match tokio::time::timeout(std::time::Duration::from_millis(80), self.recv()).await {
                Ok(Ok((_, _))) => continue,
                _ => break,
            }
        }
    }

    async fn roll_back(
        &mut self,
        name: &'static str,
        before: &Value,
        fields: &[String],
        backup: &Path,
        cause: SettingsError,
        window: std::time::Duration,
    ) -> SettingsError {
        let backup = backup.display().to_string();
        self.discard_late_replies().await;
        match self.commit_block(name, before, window).await {
            Ok(restored_block) => {
                let drift = describe(&compare(fields, before, before, &restored_block));
                if drift.is_empty() {
                    SettingsError::WriteFailed {
                        block: name.to_string(),
                        backup,
                        detail: cause.to_string(),
                    }
                } else {
                    SettingsError::RevertFailed {
                        block: name.to_string(),
                        backup,
                        detail: format!("{cause}; the copy was written back but {drift}"),
                    }
                }
            }
            Err(error) => SettingsError::RevertFailed {
                block: name.to_string(),
                backup,
                detail: format!("{cause}; the revert then failed: {error}"),
            },
        }
    }

    pub async fn write_setting(
        &mut self,
        name: &'static str,
        desired: Value,
        fields: &[String],
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let before = self.setting_block_within(name, write.window()).await?;
        let backup = backup_file(name, &before, write.dir())?;
        let read_back = match self.commit_block(name, &desired, write.window()).await {
            Ok(value) => value,
            Err(error) => {
                return Err(self
                    .roll_back(name, &before, fields, &backup, error, write.window())
                    .await)
            }
        };
        let compared = compare(fields, &before, &desired, &read_back);
        if compared.iter().all(|field| field.matches) {
            return Ok(WriteReport {
                block: name.to_string(),
                backup: backup.display().to_string(),
                fields: compared,
                written: redact(&desired),
                read_back: redact(&read_back),
                reverted: false,
                restored: Vec::new(),
            });
        }
        match self.commit_block(name, &before, write.window()).await {
            Ok(restored_block) => {
                let restored = compare(fields, &before, &before, &restored_block);
                let drift = describe(&restored);
                if !drift.is_empty() {
                    return Err(SettingsError::RevertFailed {
                        block: name.to_string(),
                        backup: backup.display().to_string(),
                        detail: format!("the copy was written back but {drift}"),
                    });
                }
                Ok(WriteReport {
                    block: name.to_string(),
                    backup: backup.display().to_string(),
                    fields: compared,
                    written: redact(&desired),
                    read_back: redact(&read_back),
                    reverted: true,
                    restored,
                })
            }
            Err(error) => Err(SettingsError::RevertFailed {
                block: name.to_string(),
                backup: backup.display().to_string(),
                detail: error.to_string(),
            }),
        }
    }

    pub async fn encode_limits(&mut self, window: std::time::Duration) -> EncodeLimits {
        let body =
            json!({ "Name": "EncodeCapability", "SessionID": format!("0x{:08X}", self.session()) });
        let reply = match tokio::time::timeout(window, self.request(msg::ABILITY, body)).await {
            Ok(reply) => reply,
            Err(_) => {
                self.discard_late_replies().await;
                return EncodeLimits {
                    max_bitrate: Some(EncodeLimits::APP_MAX_BITRATE),
                };
            }
        };
        let block = match reply {
            Ok(value) => value
                .get("EncodeCapability")
                .cloned()
                .unwrap_or(Value::Null),
            Err(_) => Value::Null,
        };
        let (entry, _) = first_entry(&block);
        EncodeLimits {
            max_bitrate: number(entry, "MaxBitrate").filter(|value| *value > 0),
        }
    }

    pub async fn encode_state(&mut self) -> Result<EncodeState, SettingsError> {
        let block = self.setting_block(ENCODE_BLOCK).await?;
        Ok(EncodeState {
            main: VideoFormat::read(&block, "MainFormat"),
            extra: VideoFormat::read(&block, "ExtraFormat"),
            snap: VideoFormat::read(&block, "SnapFormat"),
        })
    }

    pub async fn encode_state_within(
        &mut self,
        window: std::time::Duration,
    ) -> Result<EncodeState, SettingsError> {
        match tokio::time::timeout(window, self.encode_state()).await {
            Ok(result) => result,
            Err(_) => {
                self.discard_late_replies().await;
                Err(SettingsError::Slow {
                    block: ENCODE_BLOCK,
                    detail: "the read after the write never came".to_string(),
                })
            }
        }
    }

    pub async fn write_encode(
        &mut self,
        stream: Stream,
        patch: &EncodePatch,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let limits = self.encode_limits(write.window()).await;
        patch.validate(limits)?;
        let before = self
            .setting_block_within(ENCODE_BLOCK, write.window())
            .await?;
        let mut desired = before.clone();
        let (_, prefix) = first_entry(&before);
        let base = format!("{prefix}{}.Video", stream.key());
        let mut fields = Vec::new();
        {
            let video = slot(&mut desired, &base).ok_or_else(|| SettingsError::Missing {
                block: ENCODE_BLOCK,
                path: base.clone(),
            })?;
            if let Some(fps) = patch.fps {
                video["FPS"] = json!(fps);
                fields.push(format!("{base}.FPS"));
            }
            if let Some(bitrate) = patch.bitrate {
                video["BitRate"] = json!(bitrate);
                fields.push(format!("{base}.BitRate"));
            }
            if let Some(gop) = patch.gop {
                video["GOP"] = json!(gop);
                fields.push(format!("{base}.GOP"));
            }
            if let Some(resolution) = patch.resolution.as_deref() {
                video["Resolution"] = json!(resolution.trim());
                fields.push(format!("{base}.Resolution"));
            }
        }
        self.write_setting(ENCODE_BLOCK, desired, &fields, write)
            .await
    }

    pub async fn network_state(&mut self) -> Result<NetworkState, SettingsError> {
        let block = self.setting_block(NETWORK_BLOCK).await?;
        let (common, _) = first_entry(&block);
        let dhcp = self.setting_block(DHCP_BLOCK).await.ok().and_then(|value| {
            let (entry, _) = first_entry(&value);
            boolean(entry, "Enable")
        });
        let dns = self.setting_block(DNS_BLOCK).await.ok();
        Ok(NetworkState {
            host_ip: text(common, "HostIP"),
            submask: text(common, "SubMask"),
            gateway: text(common, "GateWay"),
            mac: text(common, "MAC"),
            dhcp,
            dns: dns.as_ref().and_then(|value| {
                let (entry, _) = first_entry(value);
                text(entry, "Address")
            }),
            spare_dns: dns.as_ref().and_then(|value| {
                let (entry, _) = first_entry(value);
                text(entry, "SpareAddress")
            }),
            block: redact(&block),
        })
    }

    pub async fn write_network(
        &mut self,
        patch: &NetworkPatch,
        write: &WriteWindow,
    ) -> Result<Vec<WriteReport>, SettingsError> {
        let mut reports = Vec::new();
        if patch.dns.is_some() || patch.spare_dns.is_some() {
            let before = self.setting_block_within(DNS_BLOCK, write.window()).await?;
            let mut desired = before.clone();
            let (_, prefix) = first_entry(&before);
            let mut fields = Vec::new();
            if let Some(address) = patch.dns.as_deref() {
                let key = format!("{prefix}Address");
                if let Some(entry) = slot(&mut desired, &key) {
                    *entry = json!(address.trim());
                    fields.push(key);
                }
            }
            if let Some(address) = patch.spare_dns.as_deref() {
                let key = format!("{prefix}SpareAddress");
                if let Some(entry) = slot(&mut desired, &key) {
                    *entry = json!(address.trim());
                    fields.push(key);
                }
            }
            if fields.is_empty() {
                return Err(SettingsError::Missing {
                    block: DNS_BLOCK,
                    path: "Address".to_string(),
                });
            }
            reports.push(
                self.write_setting(DNS_BLOCK, desired, &fields, write)
                    .await?,
            );
        }
        Ok(reports)
    }

    pub async fn storage_state(&mut self) -> Result<StorageState, SettingsError> {
        let raw = match self.system_block(STORAGE_BLOCK).await {
            Ok(value) => value,
            Err(SettingsError::Device(DvrError::Rejected { .. })) => Value::Null,
            Err(error) => return Err(error),
        };
        let first = if raw.is_array() {
            raw.get(0).cloned().unwrap_or(Value::Null)
        } else {
            raw
        };
        let partitions = first
            .get("Partition")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| Partition {
                        index,
                        current: boolean(item, "IsCurrent"),
                        status: number(item, "Status"),
                        total_space: text(item, "TotalSpace"),
                        remain_space: text(item, "RemainSpace"),
                        start: text(item, "OldStartTime"),
                        end: text(item, "OldEndTime"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let overwrite = self
            .setting_block(GENERAL_BLOCK)
            .await
            .ok()
            .and_then(|value| {
                let (entry, _) = first_entry(&value);
                text(entry, "OverWrite")
            });
        let record_mode = self
            .setting_block(RECORD_BLOCK)
            .await
            .ok()
            .and_then(|value| {
                let (entry, _) = first_entry(&value);
                text(entry, "RecordMode")
            });
        Ok(StorageState {
            partitions,
            overwrite,
            record_mode,
        })
    }

    pub async fn storage_state_within(
        &mut self,
        window: std::time::Duration,
    ) -> Result<StorageState, SettingsError> {
        match tokio::time::timeout(window, self.storage_state()).await {
            Ok(result) => result,
            Err(_) => {
                self.discard_late_replies().await;
                Err(SettingsError::Slow {
                    block: STORAGE_BLOCK,
                    detail: "the read after the write never came".to_string(),
                })
            }
        }
    }

    pub async fn write_overwrite(
        &mut self,
        mode: OverwriteMode,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let before = self
            .setting_block_within(GENERAL_BLOCK, write.window())
            .await?;
        let mut desired = before.clone();
        let (_, prefix) = first_entry(&before);
        let key = format!("{prefix}OverWrite");
        {
            let entry = slot(&mut desired, &key).ok_or_else(|| SettingsError::Missing {
                block: GENERAL_BLOCK,
                path: key.clone(),
            })?;
            *entry = json!(mode.wire());
        }
        self.write_setting(GENERAL_BLOCK, desired, &[key], write)
            .await
    }

    pub async fn write_record_mode(
        &mut self,
        mode: super::control::RecordMode,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let before = self
            .setting_block_within(RECORD_BLOCK, write.window())
            .await?;
        let mut desired = before.clone();
        let (_, prefix) = first_entry(&before);
        let key = format!("{prefix}RecordMode");
        {
            let entry = slot(&mut desired, &key).ok_or_else(|| SettingsError::Missing {
                block: RECORD_BLOCK,
                path: key.clone(),
            })?;
            *entry = json!(mode.wire());
        }
        self.write_setting(RECORD_BLOCK, desired, &[key], write)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_block() -> Value {
        json!({
            "MainFormat": {
                "VideoEnable": true,
                "Video": {
                    "BitRate": 1536,
                    "BitRateControl": "CBR",
                    "Compression": "H.264",
                    "FPS": 12,
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

    #[test]
    fn dotted_paths_reach_objects_and_array_entries() {
        let block = json!({ "Record": [ { "RecordMode": "ConfigRecord" } ] });
        assert_eq!(at(&block, "Record.0.RecordMode").unwrap(), "ConfigRecord");
        assert_eq!(at(&block, "Record.1.RecordMode"), None);
        assert_eq!(at(&block, "Nope.0"), None);
    }

    #[test]
    fn numeric_fields_compare_across_integer_and_float_shapes() {
        assert!(same(&json!(12), &json!(12.0)));
        assert!(!same(&json!(12), &json!(15)));
        assert!(same(&json!(" 3072*2048 "), &json!("3072*2048")));
        assert!(!same(&json!("H.264"), &json!("H.265")));
    }

    #[test]
    fn credentials_never_leave_the_device_block() {
        let block = json!({
            "HostIP": "192.168.0.100",
            "UserName": "admin",
            "Password": "secret",
            "Nested": { "Token": "abc", "Port": 34567 }
        });
        let masked = redact(&block);
        assert_eq!(masked["UserName"], REDACTED);
        assert_eq!(masked["Password"], REDACTED);
        assert_eq!(masked["Nested"]["Token"], REDACTED);
        assert_eq!(masked["Nested"]["Port"], 34567);
        assert_eq!(masked["HostIP"], "192.168.0.100");
    }

    #[test]
    fn a_stream_patch_lands_on_the_requested_format_only() {
        let mut block = encode_block();
        {
            let video = slot(&mut block, "ExtraFormat.Video").unwrap();
            video["FPS"] = json!(8);
        }
        assert_eq!(at(&block, "ExtraFormat.Video.FPS").unwrap(), 8);
        assert_eq!(at(&block, "MainFormat.Video.FPS").unwrap(), 12);
    }

    #[test]
    fn field_comparison_reports_the_prior_requested_and_read_back_values() {
        let prior = encode_block();
        let mut desired = encode_block();
        desired["MainFormat"]["Video"]["FPS"] = json!(10);
        desired["MainFormat"]["Video"]["BitRate"] = json!(1024);
        let mut read_back = desired.clone();
        read_back["MainFormat"]["Video"]["BitRate"] = json!(1000);
        let changes = compare(
            &[
                "MainFormat.Video.FPS".to_string(),
                "MainFormat.Video.BitRate".to_string(),
            ],
            &prior,
            &desired,
            &read_back,
        );
        assert!(changes[0].matches);
        assert!(!changes[1].matches);
        assert_eq!(changes[1].before, json!(1536));
        assert_eq!(changes[1].requested, json!(1024));
        assert_eq!(changes[1].after, json!(1000));
    }

    #[test]
    fn patches_outside_the_app_and_device_bounds_are_refused_before_any_write() {
        let limits = EncodeLimits::default();
        let patch = EncodePatch {
            fps: Some(0),
            ..EncodePatch::default()
        };
        assert!(matches!(
            patch.validate(limits),
            Err(SettingsError::Invalid { field: "fps", .. })
        ));
        let patch = EncodePatch {
            bitrate: Some(-4),
            ..EncodePatch::default()
        };
        assert!(matches!(
            patch.validate(limits),
            Err(SettingsError::Invalid {
                field: "bitrate",
                ..
            })
        ));
        let patch = EncodePatch {
            bitrate: Some(999_999),
            ..EncodePatch::default()
        };
        assert!(matches!(
            patch.validate(limits),
            Err(SettingsError::Invalid {
                field: "bitrate",
                ..
            })
        ));
        let capped = EncodeLimits {
            max_bitrate: Some(2048),
        };
        let patch = EncodePatch {
            bitrate: Some(4096),
            ..EncodePatch::default()
        };
        match patch.validate(capped) {
            Err(SettingsError::Invalid { detail, .. }) => {
                assert!(detail.contains("2048"), "{detail}");
            }
            other => panic!("expected the device cap to refuse 4096, got {other:?}"),
        }
        assert!(patch
            .clone()
            .validate(EncodeLimits {
                max_bitrate: Some(8192)
            })
            .is_ok());
        let empty = EncodePatch::default().validate(limits);
        assert!(matches!(
            empty,
            Err(SettingsError::Invalid {
                field: "encode",
                ..
            })
        ));
    }

    #[test]
    fn modes_parse_from_the_words_the_route_accepts() {
        assert_eq!(
            OverwriteMode::parse("overwrite"),
            Some(OverwriteMode::Overwrite)
        );
        assert_eq!(
            OverwriteMode::parse("stop_record"),
            Some(OverwriteMode::Stop)
        );
        assert_eq!(OverwriteMode::parse("sometimes"), None);
        assert_eq!(Stream::parse("sub"), Some(Stream::Extra));
        assert_eq!(Stream::parse("main"), Some(Stream::Main));
        assert_eq!(Stream::parse("third"), None);
    }

    #[test]
    fn a_backup_name_is_a_flat_file_for_any_block_name() {
        assert_eq!(slug("Simplify.Encode"), "simplify-encode");
        assert_eq!(slug("NetWork.NetCommon"), "network-netcommon");
    }
}
