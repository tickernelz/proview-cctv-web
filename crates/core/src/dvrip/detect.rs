use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::client::{require_ok, Connection, DvrError};
use super::msg;
use super::settings::{SettingsError, WriteReport, WriteWindow};

pub const VOICE_TIPS: &str = "Ability.VoiceTipType";
pub const RULE_LIMIT: &str = "HumanRuleLimit";
pub const REMOTE_CTRL: &str = "OPRemoteCtrl";
pub const COORDINATE_MAX: i64 = 8192;
pub const PUSH_INTERVAL: (i64, i64) = (30, 1800);
pub const MANUAL_STOP: [&str; 2] = ["AlarmFunction", "ManuIntellAlertAlarm"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detector {
    Motion,
    Human,
    Car,
    Pet,
    Cry,
    Volume,
    Intell,
    RemoteCall,
    Track,
    Push,
}

impl Detector {
    pub const ALL: [Detector; 10] = [
        Detector::Motion,
        Detector::Human,
        Detector::Car,
        Detector::Pet,
        Detector::Cry,
        Detector::Volume,
        Detector::Intell,
        Detector::RemoteCall,
        Detector::Track,
        Detector::Push,
    ];

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|detector| detector.key() == raw)
    }

    pub fn key(self) -> &'static str {
        match self {
            Detector::Motion => "motion",
            Detector::Human => "human",
            Detector::Car => "car",
            Detector::Pet => "pet",
            Detector::Cry => "cry",
            Detector::Volume => "volume",
            Detector::Intell => "intell",
            Detector::RemoteCall => "remote_call",
            Detector::Track => "track",
            Detector::Push => "push",
        }
    }

    pub fn config(self) -> &'static str {
        match self {
            Detector::Motion => "Detect.MotionDetect.[0]",
            Detector::Human => "Detect.HumanDetection.[0]",
            Detector::Car => "Detect.CarShapeDetection.[0]",
            Detector::Pet => "Detect.PetDetection.[0]",
            Detector::Cry => "Detect.CryDetection.[0]",
            Detector::Volume => "Detect.VolumeDetection.[0]",
            Detector::Intell => "Alarm.IntellAlertAlarm",
            Detector::RemoteCall => "Alarm.RemoteCall",
            Detector::Track => "Detect.DetectTrack",
            Detector::Push => "NetWork.PMS",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Detector::Motion => "Motion detection",
            Detector::Human => "Human detection",
            Detector::Car => "Vehicle detection",
            Detector::Pet => "Pet detection",
            Detector::Cry => "Baby-cry detection",
            Detector::Volume => "Abnormal-sound detection",
            Detector::Intell => "Intelligent vigilance",
            Detector::RemoteCall => "One-key call",
            Detector::Track => "Human tracking",
            Detector::Push => "Device push service",
        }
    }

    pub fn ability(self) -> Option<[&'static str; 2]> {
        match self {
            Detector::Motion | Detector::Push => None,
            Detector::Human => Some(["AlarmFunction", "PEAInHumanPed"]),
            Detector::Car => Some(["AlarmFunction", "CarShapeDetection"]),
            Detector::Pet => Some(["AlarmFunction", "HumanBasedPetDetect"]),
            Detector::Cry => Some(["AlarmFunction", "CryDetection"]),
            Detector::Volume => Some(["OtherFunction", "SupportVolumeDetect"]),
            Detector::Intell => Some(["AlarmFunction", "IntellAlertAlarm"]),
            Detector::RemoteCall => Some(["OtherFunction", "SupportAlarmRemoteCall"]),
            Detector::Track => Some(["OtherFunction", "SupportDetectTrack"]),
        }
    }

    fn sensitivity_range(self) -> (i64, i64) {
        match self {
            Detector::Track => (0, 2),
            Detector::Volume => (70, 110),
            _ => (0, 100),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Gate {
    pub supported: bool,
    pub evidence: String,
}

impl Gate {
    fn yes(evidence: impl Into<String>) -> Self {
        Self {
            supported: true,
            evidence: evidence.into(),
        }
    }

    fn no(evidence: impl Into<String>) -> Self {
        Self {
            supported: false,
            evidence: evidence.into(),
        }
    }
}

pub fn gate(abilities: &Value, path: [&str; 2]) -> Gate {
    let shown = path.join(".");
    match abilities
        .get(path[0])
        .and_then(|group| group.get(path[1]))
        .and_then(Value::as_bool)
    {
        Some(true) => Gate::yes(format!("device reports {shown}=true")),
        Some(false) => Gate::no(format!("device reports {shown}=false")),
        None => Gate::no(format!("device does not report {shown}")),
    }
}

pub fn detector_gate(detector: Detector, abilities: &Value) -> Gate {
    match detector.ability() {
        Some(path) => gate(abilities, path),
        None => Gate::yes(format!(
            "{} is a base ability; the app reads it without a flag",
            detector.config()
        )),
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub struct RuleLimit {
    pub area: bool,
    pub line: bool,
    pub sensitivity: bool,
    pub show_track: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Tone {
    pub id: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HumanRule {
    pub rule_type: i64,
    pub enabled: bool,
    pub area: Vec<[i64; 2]>,
    pub line: Option<[i64; 4]>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Summary {
    pub enable: Option<bool>,
    pub level: Option<i64>,
    pub sensitivity: Option<i64>,
    pub record: Option<bool>,
    pub snapshot: Option<bool>,
    pub push: Option<bool>,
    pub voice: Option<bool>,
    pub beep: Option<bool>,
    pub alarm_out: Option<bool>,
    pub tip: Option<bool>,
    pub voice_type: Option<i64>,
    pub schedule: Option<Vec<Vec<String>>>,
    pub show_track: Option<bool>,
    pub rule: Option<HumanRule>,
    pub duration: Option<i64>,
    pub remote_call: Option<bool>,
    pub push_interval: Option<i64>,
    pub push_server: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DetectorView {
    pub id: Detector,
    pub label: &'static str,
    pub config: &'static str,
    pub gate: Gate,
    pub state: Option<Summary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AlarmOverview {
    pub detectors: Vec<DetectorView>,
    pub tones_gate: Gate,
    pub tones: Vec<Tone>,
    pub rule_limit: Option<RuleLimit>,
    pub combined: Gate,
    pub manual_stop: Gate,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RulePatch {
    pub rule_type: Option<i64>,
    pub enabled: Option<bool>,
    pub area: Option<Vec<[i64; 2]>>,
    pub line: Option<[i64; 4]>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DetectorPatch {
    pub enable: Option<bool>,
    pub level: Option<i64>,
    pub sensitivity: Option<i64>,
    pub record: Option<bool>,
    pub snapshot: Option<bool>,
    pub push: Option<bool>,
    pub voice: Option<bool>,
    pub beep: Option<bool>,
    pub alarm_out: Option<bool>,
    pub tip: Option<bool>,
    pub voice_type: Option<i64>,
    pub schedule: Option<Vec<Vec<String>>>,
    pub show_track: Option<bool>,
    pub rule: Option<RulePatch>,
    pub duration: Option<i64>,
    pub remote_call: Option<bool>,
    pub push_interval: Option<i64>,
}

impl DetectorPatch {
    pub fn needs_tones(&self) -> bool {
        self.voice_type.is_some()
    }

    pub fn needs_rule_limit(&self) -> bool {
        self.rule.is_some() || self.sensitivity.is_some() || self.show_track.is_some()
    }
}

#[derive(Debug, Clone, Default)]
pub struct PatchContext {
    pub tones: Option<Vec<Tone>>,
    pub rule_limit: Option<RuleLimit>,
}

#[derive(Debug)]
pub enum DetectError {
    Unsupported {
        feature: &'static str,
        evidence: String,
    },
    Settings(SettingsError),
}

impl From<SettingsError> for DetectError {
    fn from(error: SettingsError) -> Self {
        DetectError::Settings(error)
    }
}

impl From<DvrError> for DetectError {
    fn from(error: DvrError) -> Self {
        DetectError::Settings(SettingsError::Device(error))
    }
}

impl std::fmt::Display for DetectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DetectError::Unsupported { feature, evidence } => {
                write!(f, "{feature} is not offered by this camera: {evidence}")
            }
            DetectError::Settings(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DetectError {}

const LINKAGE: [(&str, &str); 7] = [
    ("record", "RecordEnable"),
    ("snapshot", "SnapEnable"),
    ("push", "MessageEnable"),
    ("voice", "VoiceEnable"),
    ("beep", "BeepEnable"),
    ("alarm_out", "AlarmOutEnable"),
    ("tip", "TipEnable"),
];

fn at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cursor = value;
    for segment in path.split('.') {
        cursor = match cursor {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    if cursor.is_null() {
        None
    } else {
        Some(cursor)
    }
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
    if cursor.is_null() {
        None
    } else {
        Some(cursor)
    }
}

fn read_bool(value: &Value, path: &str) -> Option<bool> {
    at(value, path).and_then(|item| item.as_bool().or_else(|| item.as_i64().map(|n| n != 0)))
}

fn read_int(value: &Value, path: &str) -> Option<i64> {
    at(value, path).and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_f64().map(|float| float as i64))
            .or_else(|| item.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

fn read_schedule(value: &Value) -> Option<Vec<Vec<String>>> {
    at(value, "EventHandler.TimeSection")
        .and_then(Value::as_array)
        .map(|days| {
            days.iter()
                .map(|day| {
                    day.as_array()
                        .map(|slots| {
                            slots
                                .iter()
                                .map(|slot| slot.as_str().unwrap_or("").to_string())
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
}

fn read_rule(value: &Value) -> Option<HumanRule> {
    let rule = at(value, "PedRule.0")?;
    let area = at(rule, "RuleRegion.Pts")
        .and_then(Value::as_array)
        .map(|points| {
            points
                .iter()
                .map(|point| {
                    [
                        read_int(point, "X").unwrap_or(0),
                        read_int(point, "Y").unwrap_or(0),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    let line = at(rule, "RuleLine.Pts").map(|pts| {
        [
            read_int(pts, "StartX").unwrap_or(0),
            read_int(pts, "StartY").unwrap_or(0),
            read_int(pts, "StopX").unwrap_or(0),
            read_int(pts, "StopY").unwrap_or(0),
        ]
    });
    Some(HumanRule {
        rule_type: read_int(rule, "RuleType").unwrap_or(1),
        enabled: read_bool(rule, "Enable").unwrap_or(false),
        area,
        line,
    })
}

pub fn summarize(detector: Detector, block: &Value) -> Summary {
    let linkage = |key: &str| read_bool(block, &format!("EventHandler.{key}"));
    Summary {
        enable: read_bool(block, "Enable"),
        level: read_int(block, "Level"),
        sensitivity: read_int(block, "Sensitivity"),
        record: linkage("RecordEnable"),
        snapshot: linkage("SnapEnable"),
        push: linkage("MessageEnable"),
        voice: linkage("VoiceEnable"),
        beep: linkage("BeepEnable"),
        alarm_out: linkage("AlarmOutEnable"),
        tip: linkage("TipEnable"),
        voice_type: read_int(block, "EventHandler.VoiceType"),
        schedule: read_schedule(block),
        show_track: read_bool(block, "ShowTrack"),
        rule: if detector == Detector::Human {
            read_rule(block)
        } else {
            None
        },
        duration: read_int(block, "Duration"),
        remote_call: read_bool(block, "RemoteCallEenble"),
        push_interval: if detector == Detector::Push {
            read_int(block, "PushInterval")
        } else {
            None
        },
        push_server: if detector == Detector::Push {
            at(block, "ServName")
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            None
        },
    }
}

pub fn tones_of(reply: &Value) -> Vec<Tone> {
    at(reply, "VoiceTip")
        .and_then(Value::as_array)
        .map(|tips| {
            tips.iter()
                .filter_map(|tip| {
                    Some(Tone {
                        id: read_int(tip, "VoiceEnum")?,
                        text: at(tip, "VoiceText")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn rule_limit_of(reply: &Value) -> RuleLimit {
    RuleLimit {
        area: read_bool(reply, "SupportArea").unwrap_or(false),
        line: read_bool(reply, "SupportLine").unwrap_or(false),
        sensitivity: read_bool(reply, "Sensitivity").unwrap_or(false),
        show_track: read_bool(reply, "ShowTrack").unwrap_or(false),
    }
}

fn invalid(field: &'static str, detail: impl Into<String>) -> SettingsError {
    SettingsError::Invalid {
        field,
        detail: detail.into(),
    }
}

struct Draft<'a> {
    block: Value,
    config: &'a str,
    fields: Vec<String>,
}

impl Draft<'_> {
    fn put(&mut self, field: &'static str, path: &str, value: Value) -> Result<(), SettingsError> {
        let config = self.config;
        let target = slot(&mut self.block, path)
            .ok_or_else(|| invalid(field, format!("{config} has no {path} on this camera")))?;
        *target = value;
        if !self.fields.iter().any(|known| known == path) {
            self.fields.push(path.to_string());
        }
        Ok(())
    }

    fn flag(&mut self, field: &'static str, path: &str, on: bool) -> Result<(), SettingsError> {
        let numeric = at(&self.block, path).map(Value::is_number).unwrap_or(false);
        let value = if numeric {
            json!(i64::from(on))
        } else {
            json!(on)
        };
        self.put(field, path, value)
    }
}

fn ranged(field: &'static str, value: i64, low: i64, high: i64) -> Result<i64, SettingsError> {
    if (low..=high).contains(&value) {
        Ok(value)
    } else {
        Err(invalid(field, format!("{value} is outside {low}..={high}")))
    }
}

fn clock(raw: &str) -> Option<u32> {
    let mut parts = raw.split(':');
    let hours: u32 = parts.next()?.parse().ok()?;
    let minutes: u32 = parts.next()?.parse().ok()?;
    let seconds: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || raw.len() != 8 || minutes > 59 || seconds > 59 {
        return None;
    }
    let total = hours * 3600 + minutes * 60 + seconds;
    if total > 24 * 3600 {
        return None;
    }
    Some(total)
}

pub fn valid_slot(raw: &str) -> bool {
    let Some((flag, span)) = raw.split_once(' ') else {
        return false;
    };
    let Some((start, end)) = span.split_once('-') else {
        return false;
    };
    matches!(flag, "0" | "1")
        && matches!((clock(start), clock(end)), (Some(start), Some(end)) if start <= end)
}

fn schedule_value(
    current: Option<Vec<Vec<String>>>,
    wanted: &[Vec<String>],
) -> Result<Value, SettingsError> {
    let current =
        current.ok_or_else(|| invalid("schedule", "this block has no EventHandler.TimeSection"))?;
    if wanted.len() != current.len() {
        return Err(invalid(
            "schedule",
            format!(
                "{} days sent, the camera holds {}",
                wanted.len(),
                current.len()
            ),
        ));
    }
    for (day, (wanted, held)) in wanted.iter().zip(current.iter()).enumerate() {
        if wanted.len() != held.len() {
            return Err(invalid(
                "schedule",
                format!(
                    "day {day} has {} slots, the camera holds {}",
                    wanted.len(),
                    held.len()
                ),
            ));
        }
        if let Some(bad) = wanted.iter().find(|slot| !valid_slot(slot)) {
            return Err(invalid(
                "schedule",
                format!("day {day} slot {bad:?} is not \"0|1 HH:MM:SS-HH:MM:SS\""),
            ));
        }
    }
    Ok(json!(wanted))
}

fn point(field: &'static str, value: i64) -> Result<i64, SettingsError> {
    ranged(field, value, 0, COORDINATE_MAX)
}

fn apply_rule(
    draft: &mut Draft<'_>,
    rule: &RulePatch,
    limit: Option<RuleLimit>,
) -> Result<(), SettingsError> {
    let limit = limit.ok_or_else(|| invalid("rule", "the camera did not answer HumanRuleLimit"))?;
    if let Some(kind) = rule.rule_type {
        match kind {
            0 if limit.line => {}
            1 if limit.area => {}
            0 | 1 => {
                return Err(invalid(
                    "rule",
                    format!("HumanRuleLimit does not allow rule type {kind}"),
                ))
            }
            other => {
                return Err(invalid(
                    "rule",
                    format!("rule type {other} is not 0 (line) or 1 (area)"),
                ))
            }
        }
        draft.put("rule", "PedRule.0.RuleType", json!(kind))?;
    }
    if let Some(enabled) = rule.enabled {
        draft.flag("rule", "PedRule.0.Enable", enabled)?;
    }
    if let Some(area) = &rule.area {
        if !limit.area {
            return Err(invalid("rule", "HumanRuleLimit reports SupportArea=false"));
        }
        let held = at(&draft.block, "PedRule.0.RuleRegion.Pts")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        if area.len() != held {
            return Err(invalid(
                "rule",
                format!("{} area points sent, the camera holds {held}", area.len()),
            ));
        }
        let mut points = Vec::with_capacity(area.len());
        for [x, y] in area {
            points.push(json!({ "X": point("rule", *x)?, "Y": point("rule", *y)? }));
        }
        draft.put("rule", "PedRule.0.RuleRegion.Pts", Value::Array(points))?;
        draft.put("rule", "PedRule.0.RuleRegion.PtsNum", json!(area.len()))?;
    }
    if let Some([start_x, start_y, stop_x, stop_y]) = rule.line {
        if !limit.line {
            return Err(invalid("rule", "HumanRuleLimit reports SupportLine=false"));
        }
        draft.put(
            "rule",
            "PedRule.0.RuleLine.Pts",
            json!({
                "StartX": point("rule", start_x)?,
                "StartY": point("rule", start_y)?,
                "StopX": point("rule", stop_x)?,
                "StopY": point("rule", stop_y)?,
            }),
        )?;
    }
    Ok(())
}

pub fn apply(
    detector: Detector,
    current: &Value,
    patch: &DetectorPatch,
    context: &PatchContext,
) -> Result<(Value, Vec<String>), SettingsError> {
    let interval_only = DetectorPatch {
        push_interval: patch.push_interval,
        ..Default::default()
    };
    if detector == Detector::Push && *patch != interval_only {
        return Err(invalid(
            "patch",
            "NetWork.PMS takes push_interval only; the app edits nothing else there",
        ));
    }
    let mut draft = Draft {
        block: current.clone(),
        config: detector.config(),
        fields: Vec::new(),
    };
    if let Some(on) = patch.enable {
        draft.flag("enable", "Enable", on)?;
    }
    if let Some(level) = patch.level {
        draft.put("level", "Level", json!(ranged("level", level, 1, 6)?))?;
    }
    if let Some(sensitivity) = patch.sensitivity {
        if detector == Detector::Human
            && !context
                .rule_limit
                .map(|limit| limit.sensitivity)
                .unwrap_or(false)
        {
            return Err(invalid(
                "sensitivity",
                "HumanRuleLimit reports Sensitivity=false",
            ));
        }
        let (low, high) = detector.sensitivity_range();
        draft.put(
            "sensitivity",
            "Sensitivity",
            json!(ranged("sensitivity", sensitivity, low, high)?),
        )?;
    }
    let flags = [
        patch.record,
        patch.snapshot,
        patch.push,
        patch.voice,
        patch.beep,
        patch.alarm_out,
        patch.tip,
    ];
    for ((name, key), wanted) in LINKAGE.iter().zip(flags) {
        if let Some(on) = wanted {
            let field: &'static str = name;
            draft.flag(field, &format!("EventHandler.{key}"), on)?;
        }
    }
    if let Some(tone) = patch.voice_type {
        let tones = context.tones.as_deref().unwrap_or(&[]);
        if !tones.iter().any(|known| known.id == tone) {
            return Err(invalid(
                "voice_type",
                format!("{tone} is not in Ability.VoiceTipType"),
            ));
        }
        draft.put("voice_type", "EventHandler.VoiceType", json!(tone))?;
    }
    if let Some(schedule) = &patch.schedule {
        let value = schedule_value(read_schedule(current), schedule)?;
        draft.put("schedule", "EventHandler.TimeSection", value)?;
    }
    if let Some(on) = patch.show_track {
        if detector == Detector::Human
            && !context
                .rule_limit
                .map(|limit| limit.show_track)
                .unwrap_or(false)
        {
            return Err(invalid(
                "show_track",
                "HumanRuleLimit reports ShowTrack=false",
            ));
        }
        draft.flag("show_track", "ShowTrack", on)?;
    }
    if let Some(rule) = &patch.rule {
        if detector != Detector::Human {
            return Err(invalid("rule", "only human detection carries PedRule"));
        }
        apply_rule(&mut draft, rule, context.rule_limit)?;
    }
    if let Some(duration) = patch.duration {
        draft.put(
            "duration",
            "Duration",
            json!(ranged("duration", duration, 1, 600)?),
        )?;
    }
    if let Some(on) = patch.remote_call {
        draft.flag("remote_call", "RemoteCallEenble", on)?;
    }
    if let Some(interval) = patch.push_interval {
        if detector != Detector::Push {
            return Err(invalid(
                "push_interval",
                "only NetWork.PMS carries PushInterval",
            ));
        }
        draft.put(
            "push_interval",
            "PushInterval",
            json!(ranged(
                "push_interval",
                interval,
                PUSH_INTERVAL.0,
                PUSH_INTERVAL.1
            )?),
        )?;
    }
    if draft.fields.is_empty() {
        return Err(invalid("patch", "nothing to change"));
    }
    Ok((draft.block, draft.fields))
}

pub fn manual_stop_body(session: u32) -> Value {
    json!({
        "Name": REMOTE_CTRL,
        "SessionId": format!("0x{session:08X}"),
        "OPRemoteCtrl": {
            "Type": "ManuIntelAlarm",
            "msg": "0x00000000",
            "P1": "0x0",
            "P2": "0x0",
        },
    })
}

fn slow(block: &'static str, detail: &str) -> SettingsError {
    SettingsError::Slow {
        block,
        detail: detail.to_string(),
    }
}

fn unsupported_ret(error: &SettingsError) -> Option<i64> {
    match error {
        SettingsError::Device(DvrError::Rejected { ret, .. }) if *ret == 607 || *ret == 103 => {
            Some(*ret)
        }
        _ => None,
    }
}

impl Connection {
    async fn ability_within(
        &mut self,
        name: &'static str,
        window: std::time::Duration,
    ) -> Result<Value, SettingsError> {
        let body = json!({ "Name": name, "SessionID": format!("0x{:08X}", self.session()) });
        let reply = tokio::time::timeout(window, self.request(msg::ABILITY, body))
            .await
            .map_err(|_| slow(name, "the ability read never answered"))??;
        require_ok(&reply)?;
        Ok(reply.get(name).cloned().unwrap_or(Value::Null))
    }

    async fn block_within(
        &mut self,
        name: &'static str,
        window: std::time::Duration,
    ) -> Result<Value, SettingsError> {
        tokio::time::timeout(window, self.setting_block(name))
            .await
            .map_err(|_| slow(name, "the read never answered"))?
    }

    pub async fn alarm_abilities_within(
        &mut self,
        window: std::time::Duration,
    ) -> Result<Value, SettingsError> {
        self.ability_within("SystemFunction", window).await
    }

    async fn optional_within(
        &mut self,
        name: &'static str,
        ability: bool,
        window: std::time::Duration,
    ) -> Result<Option<Value>, SettingsError> {
        let read = if ability {
            self.ability_within(name, window).await
        } else {
            self.block_within(name, window).await
        };
        match read {
            Ok(value) => Ok(Some(value)),
            Err(SettingsError::Slow { block, detail }) => {
                Err(SettingsError::Slow { block, detail })
            }
            Err(_) => Ok(None),
        }
    }

    pub async fn alarm_overview_within(
        &mut self,
        window: std::time::Duration,
    ) -> Result<AlarmOverview, SettingsError> {
        let abilities = self.alarm_abilities_within(window).await?;
        let tones_gate = gate(&abilities, ["OtherFunction", "SupportAlarmVoiceTips"]);
        let tones = if tones_gate.supported {
            self.optional_within(VOICE_TIPS, false, window)
                .await?
                .map(|reply| tones_of(&reply))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let human = gate(&abilities, ["AlarmFunction", "PEAInHumanPed"]);
        let rule_limit = if human.supported {
            self.optional_within(RULE_LIMIT, true, window)
                .await?
                .map(|reply| rule_limit_of(&reply))
        } else {
            None
        };
        let mut detectors = Vec::with_capacity(Detector::ALL.len());
        for detector in Detector::ALL {
            let mut view = DetectorView {
                id: detector,
                label: detector.label(),
                config: detector.config(),
                gate: detector_gate(detector, &abilities),
                state: None,
                error: None,
            };
            if view.gate.supported {
                match self.block_within(detector.config(), window).await {
                    Ok(block) => view.state = Some(summarize(detector, &block)),
                    Err(error @ SettingsError::Slow { .. }) => return Err(error),
                    Err(error) => match unsupported_ret(&error) {
                        Some(ret) => {
                            view.gate = Gate::no(format!(
                                "{}; {} answered Ret {ret}",
                                view.gate.evidence,
                                detector.config()
                            ))
                        }
                        None => view.error = Some(error.to_string()),
                    },
                }
            }
            detectors.push(view);
        }
        Ok(AlarmOverview {
            detectors,
            tones_gate,
            tones,
            rule_limit,
            combined: gate(&abilities, ["AlarmFunction", "MotionHumanDection"]),
            manual_stop: gate(&abilities, MANUAL_STOP),
        })
    }

    pub async fn write_detector(
        &mut self,
        detector: Detector,
        patch: &DetectorPatch,
        write: &WriteWindow,
    ) -> Result<WriteReport, DetectError> {
        let window = write.timeout;
        let abilities = self.alarm_abilities_within(window).await?;
        let gated = detector_gate(detector, &abilities);
        if !gated.supported {
            return Err(DetectError::Unsupported {
                feature: detector.label(),
                evidence: gated.evidence,
            });
        }
        let mut context = PatchContext::default();
        if patch.needs_tones() {
            let tips = gate(&abilities, ["OtherFunction", "SupportAlarmVoiceTips"]);
            if !tips.supported {
                return Err(DetectError::Unsupported {
                    feature: "Alarm tone",
                    evidence: tips.evidence,
                });
            }
            context.tones = Some(tones_of(&self.block_within(VOICE_TIPS, window).await?));
        }
        if patch.needs_rule_limit() && detector == Detector::Human {
            context.rule_limit = Some(rule_limit_of(
                &self.ability_within(RULE_LIMIT, window).await?,
            ));
        }
        let current = match self.block_within(detector.config(), window).await {
            Ok(block) => block,
            Err(error) => {
                return Err(match unsupported_ret(&error) {
                    Some(ret) => DetectError::Unsupported {
                        feature: detector.label(),
                        evidence: format!("{} answered Ret {ret}", detector.config()),
                    },
                    None => DetectError::Settings(error),
                })
            }
        };
        let (desired, fields) = apply(detector, &current, patch, &context)?;
        Ok(self
            .write_setting(detector.config(), desired, &fields, write)
            .await?)
    }

    pub async fn manual_stop_gate_within(
        &mut self,
        window: std::time::Duration,
    ) -> Result<Gate, SettingsError> {
        let abilities = self.alarm_abilities_within(window).await?;
        Ok(gate(&abilities, MANUAL_STOP))
    }

    pub async fn stop_manual_alarm_within(
        &mut self,
        window: std::time::Duration,
    ) -> Result<i64, DetectError> {
        let abilities = self.alarm_abilities_within(window).await?;
        let gated = gate(&abilities, MANUAL_STOP);
        if !gated.supported {
            return Err(DetectError::Unsupported {
                feature: "Stop sounding alarm",
                evidence: gated.evidence,
            });
        }
        let body = manual_stop_body(self.session());
        let reply = tokio::time::timeout(window, self.request(msg::OP_REMOTE_CTRL, body))
            .await
            .map_err(|_| {
                slow(
                    REMOTE_CTRL,
                    "the stop command was sent and never acknowledged",
                )
            })?
            .map_err(DetectError::from)?;
        Ok(require_ok(&reply)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion() -> Value {
        json!({
            "AlarmType": 0,
            "Enable": true,
            "Level": 3,
            "Region": vec!["0xFFFFFFFF"; 32],
            "EventHandler": {
                "MessageEnable": true,
                "RecordEnable": true,
                "SnapEnable": true,
                "VoiceEnable": false,
                "BeepEnable": false,
                "AlarmOutEnable": false,
                "TipEnable": false,
                "VoiceType": 24,
                "TimeSection": vec![vec!["1 00:00:00-24:00:00", "0 00:00:00-24:00:00"]; 7],
            }
        })
    }

    fn human() -> Value {
        json!({
            "Enable": true,
            "Sensitivity": 1,
            "ShowTrack": true,
            "PedRule": [{
                "Enable": true,
                "RuleType": 1,
                "RuleLine": { "AlarmDirect": 2, "Pts": { "StartX": 100, "StartY": 100, "StopX": 8191, "StopY": 8191 } },
                "RuleRegion": { "AlarmDirect": 2, "PtsNum": 4, "Pts": [
                    { "X": 100, "Y": 100 }, { "X": 8191, "Y": 100 }, { "X": 8191, "Y": 8191 }, { "X": 100, "Y": 8191 }
                ] },
                "SensorIndex": 0
            }]
        })
    }

    fn tones() -> Vec<Tone> {
        tones_of(&json!({ "VoiceTip": [
            { "VoiceEnum": 24, "VoiceText": "Bell Alarm" },
            { "VoiceEnum": 523, "VoiceText": "You have entered the monitoring area" }
        ] }))
    }

    fn limit(sensitivity: bool) -> RuleLimit {
        rule_limit_of(&json!({
            "AreaNum": 1, "LineNum": 1, "Sensitivity": sensitivity, "ShowRule": false,
            "ShowTrack": true, "SupportArea": true, "SupportLine": true
        }))
    }

    fn context() -> PatchContext {
        PatchContext {
            tones: Some(tones()),
            rule_limit: Some(limit(false)),
        }
    }

    fn rejects(detector: Detector, block: &Value, patch: DetectorPatch, field: &str) {
        match apply(detector, block, &patch, &context()) {
            Err(SettingsError::Invalid { field: got, .. }) => assert_eq!(got, field),
            other => panic!("expected {field} to be refused, got {other:?}"),
        }
    }

    #[test]
    fn gates_read_true_false_and_missing_flags() {
        let abilities =
            json!({ "AlarmFunction": { "PEAInHumanPed": true, "CryDetection": false } });
        assert!(detector_gate(Detector::Human, &abilities).supported);
        let cry = detector_gate(Detector::Cry, &abilities);
        assert!(!cry.supported);
        assert!(cry.evidence.contains("AlarmFunction.CryDetection=false"));
        let pet = detector_gate(Detector::Pet, &abilities);
        assert!(!pet.supported);
        assert!(pet
            .evidence
            .contains("does not report AlarmFunction.HumanBasedPetDetect"));
        assert!(detector_gate(Detector::Motion, &abilities).supported);
        let call = detector_gate(Detector::RemoteCall, &abilities);
        assert!(call
            .evidence
            .contains("OtherFunction.SupportAlarmRemoteCall"));
    }

    #[test]
    fn motion_patch_touches_only_the_named_wire_fields() {
        let patch = DetectorPatch {
            enable: Some(false),
            level: Some(6),
            push: Some(false),
            snapshot: Some(false),
            ..Default::default()
        };
        let (desired, fields) = apply(Detector::Motion, &motion(), &patch, &context()).unwrap();
        assert_eq!(desired["Enable"], json!(false));
        assert_eq!(desired["Level"], json!(6));
        assert_eq!(desired["EventHandler"]["MessageEnable"], json!(false));
        assert_eq!(desired["EventHandler"]["SnapEnable"], json!(false));
        assert_eq!(desired["EventHandler"]["RecordEnable"], json!(true));
        assert_eq!(desired["Region"], motion()["Region"]);
        assert_eq!(
            fields,
            vec![
                "Enable",
                "Level",
                "EventHandler.SnapEnable",
                "EventHandler.MessageEnable"
            ]
        );
    }

    #[test]
    fn level_outside_one_to_six_is_refused() {
        for level in [0, 7, -1] {
            rejects(
                Detector::Motion,
                &motion(),
                DetectorPatch {
                    level: Some(level),
                    ..Default::default()
                },
                "level",
            );
        }
    }

    #[test]
    fn schedule_keeps_the_seven_day_slot_shape_and_wire_format() {
        let mut schedule = vec![
            vec![
                "1 00:00:00-24:00:00".to_string(),
                "0 00:00:00-24:00:00".to_string()
            ];
            7
        ];
        schedule[2][1] = "1 08:30:00-17:45:59".to_string();
        let patch = DetectorPatch {
            schedule: Some(schedule.clone()),
            ..Default::default()
        };
        let (desired, fields) = apply(Detector::Motion, &motion(), &patch, &context()).unwrap();
        assert_eq!(
            desired["EventHandler"]["TimeSection"][2][1],
            json!("1 08:30:00-17:45:59")
        );
        assert_eq!(fields, vec!["EventHandler.TimeSection"]);
        for bad in [
            "2 00:00:00-24:00:00",
            "1 25:00:00-26:00:00",
            "1 10:00:00-09:00:00",
            "1 00:00-24:00",
            "100:00:00-24:00:00",
            "1 24:00:01-24:00:01",
        ] {
            let mut broken = schedule.clone();
            broken[0][0] = bad.to_string();
            rejects(
                Detector::Motion,
                &motion(),
                DetectorPatch {
                    schedule: Some(broken),
                    ..Default::default()
                },
                "schedule",
            );
        }
        rejects(
            Detector::Motion,
            &motion(),
            DetectorPatch {
                schedule: Some(schedule[..6].to_vec()),
                ..Default::default()
            },
            "schedule",
        );
        let mut short = schedule;
        short[4].pop();
        rejects(
            Detector::Motion,
            &motion(),
            DetectorPatch {
                schedule: Some(short),
                ..Default::default()
            },
            "schedule",
        );
    }

    #[test]
    fn tone_must_come_from_the_voice_tip_list() {
        let (desired, _) = apply(
            Detector::Motion,
            &motion(),
            &DetectorPatch {
                voice_type: Some(523),
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
        assert_eq!(desired["EventHandler"]["VoiceType"], json!(523));
        rejects(
            Detector::Motion,
            &motion(),
            DetectorPatch {
                voice_type: Some(550),
                ..Default::default()
            },
            "voice_type",
        );
    }

    #[test]
    fn a_field_the_camera_does_not_hold_is_refused_not_invented() {
        rejects(
            Detector::Human,
            &human(),
            DetectorPatch {
                push: Some(true),
                ..Default::default()
            },
            "push",
        );
        rejects(
            Detector::Intell,
            &json!({ "Enable": false, "Level": null }),
            DetectorPatch {
                level: Some(3),
                ..Default::default()
            },
            "level",
        );
        rejects(
            Detector::Motion,
            &motion(),
            DetectorPatch::default(),
            "patch",
        );
    }

    #[test]
    fn integer_flags_stay_integers() {
        let track = json!({ "Enable": 0, "ReturnTime": 300, "Sensitivity": 2 });
        let (desired, _) = apply(
            Detector::Track,
            &track,
            &DetectorPatch {
                enable: Some(true),
                sensitivity: Some(1),
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
        assert_eq!(desired["Enable"], json!(1));
        assert_eq!(desired["Sensitivity"], json!(1));
        rejects(
            Detector::Track,
            &track,
            DetectorPatch {
                sensitivity: Some(3),
                ..Default::default()
            },
            "sensitivity",
        );
    }

    #[test]
    fn human_rule_area_writes_points_and_count_inside_the_8192_space() {
        let area = vec![[0, 0], [4096, 0], [4096, 4096], [0, 4096]];
        let patch = DetectorPatch {
            rule: Some(RulePatch {
                rule_type: Some(1),
                area: Some(area),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (desired, fields) = apply(Detector::Human, &human(), &patch, &context()).unwrap();
        assert_eq!(
            desired["PedRule"][0]["RuleRegion"]["Pts"][1],
            json!({ "X": 4096, "Y": 0 })
        );
        assert_eq!(desired["PedRule"][0]["RuleRegion"]["PtsNum"], json!(4));
        assert_eq!(
            fields,
            vec![
                "PedRule.0.RuleType",
                "PedRule.0.RuleRegion.Pts",
                "PedRule.0.RuleRegion.PtsNum"
            ]
        );
        let wide = DetectorPatch {
            rule: Some(RulePatch {
                area: Some(vec![[0, 0], [8193, 0], [8192, 8192], [0, 8192]]),
                ..Default::default()
            }),
            ..Default::default()
        };
        rejects(Detector::Human, &human(), wide, "rule");
        let three = DetectorPatch {
            rule: Some(RulePatch {
                area: Some(vec![[0, 0], [10, 0], [10, 10]]),
                ..Default::default()
            }),
            ..Default::default()
        };
        rejects(Detector::Human, &human(), three, "rule");
        let line = DetectorPatch {
            rule: Some(RulePatch {
                rule_type: Some(0),
                line: Some([1, 2, 3, 4]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (desired, _) = apply(Detector::Human, &human(), &line, &context()).unwrap();
        assert_eq!(
            desired["PedRule"][0]["RuleLine"]["Pts"],
            json!({ "StartX": 1, "StartY": 2, "StopX": 3, "StopY": 4 })
        );
        assert_eq!(desired["PedRule"][0]["RuleType"], json!(0));
    }

    #[test]
    fn human_sensitivity_follows_the_rule_limit() {
        rejects(
            Detector::Human,
            &human(),
            DetectorPatch {
                sensitivity: Some(50),
                ..Default::default()
            },
            "sensitivity",
        );
        let open = PatchContext {
            rule_limit: Some(limit(true)),
            ..context()
        };
        let (desired, _) = apply(
            Detector::Human,
            &human(),
            &DetectorPatch {
                sensitivity: Some(50),
                ..Default::default()
            },
            &open,
        )
        .unwrap();
        assert_eq!(desired["Sensitivity"], json!(50));
    }

    #[test]
    fn remote_call_uses_the_vendor_spelling() {
        let block = json!({ "RemoteCallEenble": false });
        let (desired, fields) = apply(
            Detector::RemoteCall,
            &block,
            &DetectorPatch {
                remote_call: Some(true),
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
        assert_eq!(desired, json!({ "RemoteCallEenble": true }));
        assert_eq!(fields, vec!["RemoteCallEenble"]);
    }

    #[test]
    fn push_interval_is_only_written_on_the_pms_block() {
        let pms = json!({ "BoxID": "", "Enable": true, "Port": 80, "PushInterval": 10, "ServName": "push.example.invalid" });
        let (desired, _) = apply(
            Detector::Push,
            &pms,
            &DetectorPatch {
                push_interval: Some(60),
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
        assert_eq!(desired["PushInterval"], json!(60));
        assert_eq!(desired["Enable"], json!(true));
        for interval in [29, 1801, 0] {
            rejects(
                Detector::Push,
                &pms,
                DetectorPatch {
                    push_interval: Some(interval),
                    ..Default::default()
                },
                "push_interval",
            );
        }
        let edge = DetectorPatch {
            push_interval: Some(30),
            ..Default::default()
        };
        assert_eq!(
            apply(Detector::Push, &pms, &edge, &context()).unwrap().0["PushInterval"],
            json!(30)
        );
        for patch in [
            DetectorPatch {
                enable: Some(false),
                ..Default::default()
            },
            DetectorPatch {
                enable: Some(false),
                push_interval: Some(60),
                ..Default::default()
            },
            DetectorPatch {
                push: Some(false),
                ..Default::default()
            },
        ] {
            rejects(Detector::Push, &pms, patch, "patch");
        }
        rejects(
            Detector::Motion,
            &motion(),
            DetectorPatch {
                push_interval: Some(60),
                ..Default::default()
            },
            "push_interval",
        );
        let read = summarize(Detector::Push, &pms);
        assert_eq!(read.push_interval, Some(10));
        assert_eq!(read.push_server.as_deref(), Some("push.example.invalid"));
    }

    #[test]
    fn manual_stop_body_only_ever_stops() {
        let body = manual_stop_body(0x289);
        assert_eq!(
            body,
            json!({
                "Name": "OPRemoteCtrl",
                "SessionId": "0x00000289",
                "OPRemoteCtrl": { "Type": "ManuIntelAlarm", "msg": "0x00000000", "P1": "0x0", "P2": "0x0" }
            })
        );
    }

    #[test]
    fn ability_replies_parse_into_tones_and_limits() {
        assert_eq!(tones().len(), 2);
        assert_eq!(tones()[1].id, 523);
        let parsed = limit(false);
        assert!(parsed.area && parsed.line && parsed.show_track && !parsed.sensitivity);
        let summary = summarize(Detector::Human, &human());
        let rule = summary.rule.unwrap();
        assert_eq!(rule.area.len(), 4);
        assert_eq!(rule.line, Some([100, 100, 8191, 8191]));
        assert_eq!(summary.sensitivity, Some(1));
        assert_eq!(summary.push, None);
    }

    #[test]
    fn schedule_slots_accept_the_full_day() {
        assert!(valid_slot("1 00:00:00-24:00:00"));
        assert!(valid_slot("0 00:00:00-23:59:59"));
        assert!(!valid_slot("1 00:00:00 24:00:00"));
    }
}
