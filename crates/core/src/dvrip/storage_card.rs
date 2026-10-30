use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use super::client::{require_ok, Connection, DvrError};
use super::msg;
use super::settings::{
    boolean, first_entry, number, redact, slot, text, SettingsError, Stream, WriteReport,
    WriteWindow, ENCODE_BLOCK, RECORD_BLOCK,
};

pub const STORAGE_MISSING_BLOCK: &str = "Storage.StorageNotExist";
pub const RECORD_STREAM_BLOCK: &str = "Storage.RecordStream";
pub const SPLICING_BLOCK: &str = "Storage.MultiLensSplicing";
pub const STORAGE_MANAGER: &str = "OPStorageManager";
pub const FULL_DAY: &str = "1 00:00:00-24:00:00";
pub const PACKET_MINUTES: std::ops::RangeInclusive<i64> = 5..=120;
pub const PERCENT_VIEW: &str = "PreviewFunction.StorageSpaceUsePercent";
pub const RECORD_STREAM_FEATURE: &str = "OtherFunction.SupportRecMainOrExtUseMainType";
pub const SPLICING_FEATURE: &str = "OtherFunction.SupportMultiLensSplicingWfsRecordStream";
const FULL_TIME_MASK: u32 = 0x7;
const ALARM_MASK: u32 = 0x6;
const DAYS: usize = 7;

pub fn feature(system_function: &Value, path: &str) -> bool {
    super::settings::at(system_function, path)
        .map(|value| match value {
            Value::Bool(flag) => *flag,
            Value::Number(number) => number.as_f64().map(|n| n > 0.0).unwrap_or(false),
            Value::String(raw) => parse_hex(raw) > 0,
            _ => false,
        })
        .unwrap_or(false)
}

pub fn parse_hex(raw: &str) -> u64 {
    let trimmed = raw.trim();
    match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap_or(0),
        None => trimmed.parse().unwrap_or(0),
    }
}

fn space(value: &Value, key: &str) -> u64 {
    match value.get(key) {
        Some(Value::String(raw)) => parse_hex(raw),
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        _ => 0,
    }
}

fn disks(raw: &Value) -> Vec<&Value> {
    match raw {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![raw],
        _ => Vec::new(),
    }
}

fn partitions_of(disk: &Value) -> &[Value] {
    disk.get("Partition")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Partition {
    pub disk: usize,
    pub index: usize,
    pub current: Option<bool>,
    pub status: Option<i64>,
    pub driver_type: Option<i64>,
    pub total_space: Option<String>,
    pub remain_space: Option<String>,
    pub total_mb: u64,
    pub remain_mb: u64,
    pub start: Option<String>,
    pub end: Option<String>,
}

pub fn partitions(raw: &Value) -> Vec<Partition> {
    disks(raw)
        .into_iter()
        .enumerate()
        .flat_map(|(disk, entry)| {
            partitions_of(entry)
                .iter()
                .enumerate()
                .map(move |(index, item)| Partition {
                    disk,
                    index,
                    current: boolean(item, "IsCurrent"),
                    status: number(item, "Status"),
                    driver_type: number(item, "DirverType"),
                    total_space: text(item, "TotalSpace"),
                    remain_space: text(item, "RemainSpace"),
                    total_mb: space(item, "TotalSpace"),
                    remain_mb: space(item, "RemainSpace"),
                    start: text(item, "OldStartTime"),
                    end: text(item, "OldEndTime"),
                })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CardUsage {
    pub card: &'static str,
    pub total_mb: u64,
    pub remain_mb: u64,
    pub record_mb: u64,
    pub snapshot_mb: u64,
    pub remain_percent: Option<f64>,
    pub used_percent: Option<f64>,
    pub record_percent: Option<f64>,
    pub snapshot_percent: Option<f64>,
}

fn percent(part: u64, total: u64) -> Option<f64> {
    if total == 0 {
        return None;
    }
    let value = (part as f64 * 100.0 / total as f64).min(100.0);
    Some((value * 10.0).round() / 10.0)
}

pub fn card_usage(raw: &Value) -> CardUsage {
    let mut total = 0;
    let mut remain = 0;
    let mut record = 0;
    let mut snapshot = 0;
    let mut faulty = false;
    for disk in disks(raw) {
        let declared = number(disk, "PartNumber").unwrap_or(0).max(0) as usize;
        for item in partitions_of(disk).iter().take(declared) {
            if number(item, "Status").unwrap_or(0) != 0 {
                faulty = true;
                continue;
            }
            let size = space(item, "TotalSpace");
            let free = space(item, "RemainSpace");
            match number(item, "DirverType").unwrap_or(0) {
                0 | 5 => record += size,
                4 => snapshot += size,
                _ => continue,
            }
            total += size;
            remain += free;
        }
    }
    let card = match (total, faulty) {
        (0, true) => "error",
        (0, false) => "absent",
        _ => "ok",
    };
    CardUsage {
        card,
        total_mb: total,
        remain_mb: remain,
        record_mb: record,
        snapshot_mb: snapshot,
        remain_percent: percent(remain, total),
        used_percent: percent(total.saturating_sub(remain), total),
        record_percent: percent(record, total),
        snapshot_percent: percent(snapshot, total),
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct FormatTarget {
    pub disk: usize,
    pub partition: usize,
}

pub fn format_targets(raw: &Value) -> Vec<FormatTarget> {
    disks(raw)
        .into_iter()
        .enumerate()
        .flat_map(|(disk, entry)| {
            partitions_of(entry)
                .iter()
                .enumerate()
                .filter(|(_, item)| space(item, "TotalSpace") != 0)
                .map(move |(partition, _)| FormatTarget { disk, partition })
        })
        .collect()
}

pub fn format_request(target: FormatTarget, session: u32) -> Value {
    json!({
        "Name": STORAGE_MANAGER,
        STORAGE_MANAGER: {
            "Action": "Clear",
            "PartNo": target.partition,
            "SerialNo": target.disk,
            "Type": "Data",
        },
        "SessionID": format!("0x{session:08X}"),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct FormatStep {
    pub disk: usize,
    pub partition: usize,
    pub request: Value,
    pub ret: i64,
    pub seconds: f64,
}

pub fn mask_text(existing: Option<&str>, bits: u32) -> String {
    match existing.map(str::trim) {
        Some(raw) if raw.len() == 10 && raw.starts_with("0x") => format!("0x{bits:08X}"),
        Some(raw) if raw.starts_with("0X") => format!("0X{bits:08X}"),
        _ => format!("0x{bits:x}"),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordState {
    pub mode: Option<String>,
    pub full_time: Option<bool>,
    pub packet_length: Option<i64>,
    pub pre_record: Option<i64>,
    pub schedule: Vec<String>,
    pub masks: Vec<String>,
    pub record_stream: Option<String>,
    pub record_stream_supported: bool,
    pub splicing: Option<bool>,
    pub splicing_supported: bool,
    pub audio: Option<bool>,
    pub block: Value,
}

fn first_cells(entry: &Value, key: &str) -> Vec<String> {
    entry
        .get(key)
        .and_then(Value::as_array)
        .map(|days| {
            days.iter()
                .map(|day| {
                    day.get(0)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn full_time(entry: &Value) -> Option<bool> {
    let mode = text(entry, "RecordMode")?;
    Some(match mode.as_str() {
        "ConfigRecord" => first_cells(entry, "Mask")
            .first()
            .map(|cell| parse_hex(cell) == u64::from(FULL_TIME_MASK))
            .unwrap_or(false),
        "ManualRecord" => true,
        _ => false,
    })
}

pub fn record_stream_label(value: &Value) -> Option<String> {
    let (entry, _) = first_entry(value);
    match number(entry, "MainStream")? {
        0 => Some("main".to_string()),
        1 => Some("extra".to_string()),
        other => Some(other.to_string()),
    }
}

pub fn schedule_patch(before: &Value, full: bool) -> Result<(Value, Vec<String>), SettingsError> {
    let mut desired = before.clone();
    let (entry, prefix) = first_entry(before);
    let existing = first_cells(entry, "Mask");
    let bits = if full { FULL_TIME_MASK } else { ALARM_MASK };
    let mut fields = Vec::new();
    if entry.get("RecordMode").is_some() {
        let key = format!("{prefix}RecordMode");
        if let Some(slot) = slot(&mut desired, &key) {
            *slot = json!("ConfigRecord");
        }
        fields.push(key);
    }
    for (key, value) in [
        (
            "Mask",
            mask_text(existing.first().map(String::as_str), bits),
        ),
        ("TimeSection", FULL_DAY.to_string()),
    ] {
        let mut touched = false;
        for day in 0..DAYS {
            if let Some(cell) = slot(&mut desired, &format!("{prefix}{key}.{day}.0")) {
                *cell = json!(value);
                touched = true;
            }
        }
        if !touched {
            return Err(SettingsError::Missing {
                block: RECORD_BLOCK,
                path: format!("{prefix}{key}"),
            });
        }
        fields.push(format!("{prefix}{key}"));
    }
    Ok((desired, fields))
}

fn set_field(
    block: &'static str,
    before: &Value,
    path: &str,
    value: Value,
) -> Result<(Value, String), SettingsError> {
    let mut desired = before.clone();
    let (_, prefix) = first_entry(before);
    let key = format!("{prefix}{path}");
    let entry = slot(&mut desired, &key).ok_or_else(|| SettingsError::Missing {
        block,
        path: key.clone(),
    })?;
    *entry = value;
    Ok((desired, key))
}

impl Connection {
    pub async fn system_function(&mut self) -> Value {
        self.abilities().await.unwrap_or(Value::Null)
    }

    pub async fn storage_raw(&mut self) -> Result<Value, SettingsError> {
        match self.system_block(super::settings::STORAGE_BLOCK).await {
            Ok(value) => Ok(value),
            Err(SettingsError::Device(DvrError::Rejected { .. })) => Ok(Value::Null),
            Err(error) => Err(error),
        }
    }

    pub async fn storage_missing_alarm(&mut self) -> Option<bool> {
        let block = self.setting_block(STORAGE_MISSING_BLOCK).await.ok()?;
        let (entry, _) = first_entry(&block);
        boolean(entry, "Enable")
    }

    pub async fn write_storage_missing_alarm(
        &mut self,
        enable: bool,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let before = self
            .setting_block_within(STORAGE_MISSING_BLOCK, write.window())
            .await?;
        let (desired, key) = set_field(STORAGE_MISSING_BLOCK, &before, "Enable", json!(enable))?;
        self.write_setting(STORAGE_MISSING_BLOCK, desired, &[key], write)
            .await
    }

    pub async fn format_card(
        &mut self,
        window: Duration,
    ) -> Result<Vec<FormatStep>, SettingsError> {
        let raw = match tokio::time::timeout(window, self.storage_raw()).await {
            Ok(result) => result?,
            Err(_) => {
                self.discard_late_replies().await;
                return Err(SettingsError::Slow {
                    block: super::settings::STORAGE_BLOCK,
                    detail: "the card read before the format never came".to_string(),
                });
            }
        };
        let targets = format_targets(&raw);
        if targets.is_empty() {
            return Err(SettingsError::Invalid {
                field: "format",
                detail: "the device reports no partition with space, so there is nothing to format"
                    .to_string(),
            });
        }
        let mut steps = Vec::new();
        for target in targets {
            let request = format_request(target, self.session());
            let started = Instant::now();
            let reply = match tokio::time::timeout(
                window,
                self.request(msg::STORAGE_MANAGER, request.clone()),
            )
            .await
            {
                Ok(reply) => reply?,
                Err(_) => {
                    self.discard_late_replies().await;
                    return Err(SettingsError::Slow {
                        block: STORAGE_MANAGER,
                        detail: format!(
                            "disk {} partition {} never acknowledged the format",
                            target.disk, target.partition
                        ),
                    });
                }
            };
            let ret = require_ok(&reply)?;
            steps.push(FormatStep {
                disk: target.disk,
                partition: target.partition,
                request,
                ret,
                seconds: started.elapsed().as_secs_f64(),
            });
        }
        Ok(steps)
    }

    pub async fn record_state(&mut self) -> Result<RecordState, SettingsError> {
        let block = self.setting_block(RECORD_BLOCK).await?;
        let (entry, _) = first_entry(&block);
        let features = self.system_function().await;
        let record_stream_supported = feature(&features, RECORD_STREAM_FEATURE);
        let splicing_supported = feature(&features, SPLICING_FEATURE);
        let record_stream = if record_stream_supported {
            self.setting_block(RECORD_STREAM_BLOCK)
                .await
                .ok()
                .and_then(|value| record_stream_label(&value))
        } else {
            None
        };
        let splicing = if splicing_supported {
            self.setting_block(SPLICING_BLOCK)
                .await
                .ok()
                .and_then(|value| boolean(first_entry(&value).0, "Enable"))
        } else {
            None
        };
        let audio = self
            .setting_block(ENCODE_BLOCK)
            .await
            .ok()
            .and_then(|value| {
                let (encode, _) = first_entry(&value);
                encode
                    .get("MainFormat")
                    .and_then(|main| boolean(main, "AudioEnable"))
            });
        Ok(RecordState {
            mode: text(entry, "RecordMode"),
            full_time: full_time(entry),
            packet_length: number(entry, "PacketLength"),
            pre_record: number(entry, "PreRecord"),
            schedule: first_cells(entry, "TimeSection"),
            masks: first_cells(entry, "Mask"),
            record_stream,
            record_stream_supported,
            splicing,
            splicing_supported,
            audio,
            block: redact(&block),
        })
    }

    pub async fn record_state_within(
        &mut self,
        window: Duration,
    ) -> Result<RecordState, SettingsError> {
        match tokio::time::timeout(window, self.record_state()).await {
            Ok(result) => result,
            Err(_) => {
                self.discard_late_replies().await;
                Err(SettingsError::Slow {
                    block: RECORD_BLOCK,
                    detail: "the read never came".to_string(),
                })
            }
        }
    }

    pub async fn write_record_schedule(
        &mut self,
        full: Option<bool>,
        packet_length: Option<i64>,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        if let Some(minutes) = packet_length {
            if !PACKET_MINUTES.contains(&minutes) {
                return Err(SettingsError::Invalid {
                    field: "packet_length",
                    detail: format!(
                        "{minutes} minutes is outside {}..{}",
                        PACKET_MINUTES.start(),
                        PACKET_MINUTES.end()
                    ),
                });
            }
        }
        let before = self
            .setting_block_within(RECORD_BLOCK, write.window())
            .await?;
        let (mut desired, mut fields) = match full {
            Some(full) => schedule_patch(&before, full)?,
            None => (before.clone(), Vec::new()),
        };
        if let Some(minutes) = packet_length {
            let (_, prefix) = first_entry(&before);
            let key = format!("{prefix}PacketLength");
            let entry = slot(&mut desired, &key).ok_or_else(|| SettingsError::Missing {
                block: RECORD_BLOCK,
                path: key.clone(),
            })?;
            *entry = json!(minutes);
            fields.push(key);
        }
        if fields.is_empty() {
            return Err(SettingsError::Invalid {
                field: "record",
                detail: "name full_time and/or packet_length".to_string(),
            });
        }
        self.write_setting(RECORD_BLOCK, desired, &fields, write)
            .await
    }

    pub async fn write_record_stream(
        &mut self,
        stream: Stream,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let features = self.system_function().await;
        if !feature(&features, RECORD_STREAM_FEATURE) {
            return Err(SettingsError::Missing {
                block: RECORD_STREAM_BLOCK,
                path: RECORD_STREAM_FEATURE.to_string(),
            });
        }
        let before = self
            .setting_block_within(RECORD_STREAM_BLOCK, write.window())
            .await?;
        let value = match stream {
            Stream::Main => 0,
            Stream::Extra => 1,
        };
        let (desired, key) = set_field(RECORD_STREAM_BLOCK, &before, "MainStream", json!(value))?;
        self.write_setting(RECORD_STREAM_BLOCK, desired, &[key], write)
            .await
    }

    pub async fn write_splicing(
        &mut self,
        enable: bool,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let features = self.system_function().await;
        if !feature(&features, SPLICING_FEATURE) {
            return Err(SettingsError::Missing {
                block: SPLICING_BLOCK,
                path: SPLICING_FEATURE.to_string(),
            });
        }
        let before = self
            .setting_block_within(SPLICING_BLOCK, write.window())
            .await?;
        let (desired, key) = set_field(SPLICING_BLOCK, &before, "Enable", json!(enable))?;
        self.write_setting(SPLICING_BLOCK, desired, &[key], write)
            .await
    }

    pub async fn write_record_audio(
        &mut self,
        enable: bool,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let before = self
            .setting_block_within(ENCODE_BLOCK, write.window())
            .await?;
        let mut desired = before.clone();
        let (_, prefix) = first_entry(&before);
        let mut fields = Vec::new();
        for format in ["MainFormat", "ExtraFormat"] {
            let key = format!("{prefix}{format}.AudioEnable");
            if let Some(entry) = slot(&mut desired, &key) {
                *entry = json!(enable);
                fields.push(key);
            }
        }
        if fields.is_empty() {
            return Err(SettingsError::Missing {
                block: ENCODE_BLOCK,
                path: format!("{prefix}MainFormat.AudioEnable"),
            });
        }
        self.write_setting(ENCODE_BLOCK, desired, &fields, write)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Value {
        json!([{
            "PartNumber": 2,
            "Partition": [
                { "DirverType": 0, "Status": 0, "TotalSpace": "0x00000064", "RemainSpace": "0x00000019" },
                { "DirverType": 4, "Status": 0, "TotalSpace": "0x00000064", "RemainSpace": "0x00000000" },
                { "DirverType": 0, "Status": 0, "TotalSpace": "0x00000400", "RemainSpace": "0x00000400" }
            ]
        }])
    }

    #[test]
    fn usage_counts_only_declared_partitions_and_splits_record_from_pictures() {
        let usage = card_usage(&card());
        assert_eq!(usage.card, "ok");
        assert_eq!(usage.total_mb, 200);
        assert_eq!(usage.record_mb, 100);
        assert_eq!(usage.snapshot_mb, 100);
        assert_eq!(usage.remain_mb, 25);
        assert_eq!(usage.remain_percent, Some(12.5));
        assert_eq!(usage.record_percent, Some(50.0));
    }

    #[test]
    fn an_empty_or_faulty_card_is_named_not_counted() {
        assert_eq!(card_usage(&Value::Null).card, "absent");
        let faulty =
            json!([{ "PartNumber": 1, "Partition": [{ "Status": 1, "TotalSpace": "0x10" }] }]);
        assert_eq!(card_usage(&faulty).card, "error");
        assert_eq!(card_usage(&faulty).remain_percent, None);
    }

    #[test]
    fn format_targets_every_partition_with_space_on_every_disk() {
        let mut disks = card();
        disks
            .as_array_mut()
            .unwrap()
            .push(json!({ "PartNumber": 1, "Partition": [{ "TotalSpace": "0x0" }, { "TotalSpace": "0x1" }] }));
        assert_eq!(
            format_targets(&disks),
            vec![
                FormatTarget {
                    disk: 0,
                    partition: 0
                },
                FormatTarget {
                    disk: 0,
                    partition: 1
                },
                FormatTarget {
                    disk: 0,
                    partition: 2
                },
                FormatTarget {
                    disk: 1,
                    partition: 1
                },
            ]
        );
        assert_eq!(
            format_request(
                FormatTarget {
                    disk: 1,
                    partition: 1
                },
                0x11
            ),
            json!({
                "Name": "OPStorageManager",
                "OPStorageManager": { "Action": "Clear", "PartNo": 1, "SerialNo": 1, "Type": "Data" },
                "SessionID": "0x00000011",
            })
        );
    }

    #[test]
    fn masks_keep_the_width_the_device_already_uses() {
        assert_eq!(mask_text(Some("0x7"), 6), "0x6");
        assert_eq!(mask_text(Some("0x00000007"), 6), "0x00000006");
        assert_eq!(mask_text(None, 7), "0x7");
    }

    #[test]
    fn full_time_follows_the_app_reading_of_mode_and_mask() {
        assert_eq!(
            full_time(&json!({ "RecordMode": "ConfigRecord", "Mask": [["0x00000007"]] })),
            Some(true)
        );
        assert_eq!(
            full_time(&json!({ "RecordMode": "ConfigRecord", "Mask": [["0x6"]] })),
            Some(false)
        );
        assert_eq!(
            full_time(&json!({ "RecordMode": "ManualRecord" })),
            Some(true)
        );
        assert_eq!(
            full_time(&json!({ "RecordMode": "ClosedRecord" })),
            Some(false)
        );
        assert_eq!(full_time(&json!({})), None);
    }

    #[test]
    fn features_read_booleans_numbers_and_hex_along_a_path() {
        let functions = json!({ "OtherFunction": { "A": true, "B": 0, "C": "0x1" } });
        assert!(feature(&functions, "OtherFunction.A"));
        assert!(!feature(&functions, "OtherFunction.B"));
        assert!(feature(&functions, "OtherFunction.C"));
        assert!(!feature(&functions, "OtherFunction.D"));
    }

    #[test]
    fn the_schedule_patch_fails_on_a_block_without_masks() {
        assert!(matches!(
            schedule_patch(&json!([{ "RecordMode": "ConfigRecord" }]), true),
            Err(SettingsError::Missing { .. })
        ));
    }
}
