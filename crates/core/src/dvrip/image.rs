use std::future::Future;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::client::{Connection, DvrError};
use super::msg;

pub const CAMERA_PARAM: &str = "Camera.Param";
pub const CAMERA_PARAM_EX: &str = "Camera.ParamEx";
pub const WHITE_LIGHT: &str = "Camera.WhiteLight";
pub const FISH_EYE: &str = "Camera.FishEye";
pub const MASK_VIDEO: &str = "General.OneKeyMaskVideo";
pub const TIMING_SLEEP: &str = "General.TimingSleep";
pub const EXTRA_STATE: &str = "FbExtraStateCtrl";
pub const SPEAKER_VOLUME: &str = "fVideo.Volume";
pub const MIC_VOLUME: &str = "fVideo.InVolumeIn";
pub const MIC_VOLUME_ALIAS: &str = "fVideo.VolumeIn";

pub const LIGHT_LEVELS: [i64; 3] = [1, 3, 5];
pub const LIGHT_DURATIONS: [i64; 6] = [5, 10, 30, 60, 90, 120];

const DAY_NIGHT: [(i64, &str, &str); 7] = [
    (0, "auto", "Auto colour"),
    (1, "color", "Colour"),
    (2, "black_white", "Black and white"),
    (3, "smart_light", "Smart alarm light"),
    (4, "white_lamp_auto", "White lamp auto"),
    (5, "ir_lamp_auto", "IR lamp auto"),
    (6, "license_plate", "License plate"),
];

#[derive(Debug)]
pub enum ImageError {
    Timeout {
        stage: &'static str,
        window: Duration,
    },
    Unsupported {
        feature: &'static str,
        evidence: String,
    },
    Invalid(String),
    Device {
        stage: &'static str,
        error: DvrError,
    },
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Timeout { stage, window } => write!(
                f,
                "{stage} did not answer inside the image settings window ({window:?})"
            ),
            ImageError::Unsupported { feature, evidence } => {
                write!(f, "{feature} is not supported by this device: {evidence}")
            }
            ImageError::Invalid(detail) => f.write_str(detail),
            ImageError::Device { stage, error } => write!(f, "{stage}: {error}"),
        }
    }
}

impl std::error::Error for ImageError {}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Feature<T> {
    pub supported: bool,
    pub evidence: String,
    pub value: Option<T>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Choice {
    pub code: i64,
    pub key: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DayNight {
    pub code: i64,
    pub key: &'static str,
    pub label: &'static str,
    pub options: Vec<Choice>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Period {
    pub enable: bool,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Trigger {
    pub level: i64,
    pub duration: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LightSchedule {
    pub mode: Option<String>,
    pub brightness: Option<i64>,
    pub period: Option<Period>,
    pub trigger: Option<Trigger>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Sleep {
    pub enable: bool,
    pub sleeping: bool,
    pub repeat_daily: bool,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Volume {
    pub left: i64,
    pub right: i64,
    pub block: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageReport {
    pub day_night: Feature<DayNight>,
    pub light_mode: Feature<LightMode>,
    pub scene: Feature<i64>,
    pub low_lux: Feature<bool>,
    pub white_light: Feature<LightSchedule>,
    pub privacy: Feature<bool>,
    pub sleep: Feature<Sleep>,
    pub status_led: Feature<bool>,
    pub voice_tip: Feature<bool>,
    pub speaker_volume: Feature<Volume>,
    pub mic_volume: Feature<Volume>,
}

#[derive(Debug, Clone, Default)]
pub struct Blocks {
    pub abilities: Value,
    pub param: Option<Value>,
    pub param_ex: Option<Value>,
    pub white_light: Option<Value>,
    pub fish_eye: Option<Value>,
    pub mask: Option<Value>,
    pub sleep: Option<Value>,
    pub extra: Option<Value>,
    pub speaker: Option<Value>,
    pub mic: Option<(&'static str, Value)>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WriteOutcome {
    pub block: &'static str,
    pub before: Value,
    pub requested: Value,
    pub after: Value,
    pub matches: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LightPatch {
    pub period: Option<bool>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub level: Option<i64>,
    pub duration: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct SleepPatch {
    pub enable: Option<bool>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub repeat_daily: Option<bool>,
    pub wake: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeTarget {
    Speaker,
    Mic,
}

impl VolumeTarget {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "speaker" => Some(Self::Speaker),
            "mic" | "microphone" => Some(Self::Mic),
            _ => None,
        }
    }
}

pub fn ability(abilities: &Value, key: &str) -> bool {
    abilities
        .get("OtherFunction")
        .and_then(|other| other.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn ability_evidence(abilities: &Value, key: &str) -> String {
    match abilities
        .get("OtherFunction")
        .and_then(|other| other.get(key))
    {
        Some(value) => format!("SystemFunction.OtherFunction.{key}={value}"),
        None => format!("SystemFunction.OtherFunction.{key} is absent"),
    }
}

pub fn present(value: Value) -> Option<Value> {
    match &value {
        Value::Null => None,
        Value::Array(items) if items.iter().all(|item| item.is_null()) => None,
        _ => Some(value),
    }
}

pub fn first(value: &Value) -> Option<&Value> {
    match value {
        Value::Array(items) => items.first().filter(|item| item.is_object()),
        Value::Object(_) => Some(value),
        _ => None,
    }
}

fn first_mut(value: &mut Value) -> Option<&mut Value> {
    match value {
        Value::Array(items) => items.first_mut().filter(|item| item.is_object()),
        Value::Object(_) => Some(value),
        _ => None,
    }
}

fn int(entry: &Value, key: &str) -> Option<i64> {
    let value = entry.get(key)?;
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    if let Some(flag) = value.as_bool() {
        return Some(i64::from(flag));
    }
    let raw = value.as_str()?.trim();
    match raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => raw.parse().ok(),
    }
}

fn flag(entry: &Value, key: &str) -> Option<bool> {
    int(entry, key).map(|value| value != 0)
}

fn clock(hour: i64, minute: i64) -> String {
    format!("{hour:02}:{minute:02}")
}

pub fn parse_clock(raw: &str) -> Result<(i64, i64), ImageError> {
    let invalid = || ImageError::Invalid(format!("time {raw:?} must be HH:MM in 24-hour form"));
    let (hour, minute) = raw.trim().split_once(':').ok_or_else(invalid)?;
    let hour: i64 = hour.parse().map_err(|_| invalid())?;
    let minute: i64 = minute.parse().map_err(|_| invalid())?;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) {
        return Err(invalid());
    }
    Ok((hour, minute))
}

pub const DOUBLE_LIGHT: &str = "SupportDoubleLightBoxCamera";

const DOUBLE_LIGHT_EVIDENCE: &str = "SupportDoubleLightBoxCamera=true, so the app's day/night picker is p048d8/n.java (MonitorActivity.java:3115): Full colour, General night vision and Double light write Camera.WhiteLight.WorkMode Auto, Close and Intelligent (n.java:135-151), never DayNightColor, which the camera derives from that mode";

pub const LIGHT_MODES: [(&str, &str); 3] = [
    ("Auto", "Full colour"),
    ("Close", "General night vision"),
    ("Intelligent", "Double light"),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModeChoice {
    pub mode: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LightMode {
    pub mode: String,
    pub label: String,
    pub options: Vec<ModeChoice>,
}

fn light_mode_label(mode: &str) -> Option<&'static str> {
    LIGHT_MODES
        .iter()
        .find(|(wire, _)| *wire == mode)
        .map(|(_, label)| *label)
}

fn light_mode_options(entry: &Value, current: &str) -> Vec<ModeChoice> {
    let trigger = entry.get("MoveTrigLight").is_some_and(Value::is_object);
    LIGHT_MODES
        .iter()
        .filter(|(wire, _)| *wire != "Intelligent" || trigger || *wire == current)
        .map(|(wire, label)| ModeChoice {
            mode: wire.to_string(),
            label: label.to_string(),
        })
        .collect()
}

pub fn day_night_choice(code: i64) -> Option<Choice> {
    DAY_NIGHT
        .iter()
        .find(|(known, _, _)| *known == code)
        .map(|(code, key, label)| Choice {
            code: *code,
            key,
            label,
        })
}

pub fn day_night_options(abilities: &Value, current: Option<i64>) -> Vec<Choice> {
    let mut codes = vec![0, 1, 2];
    if ability(abilities, "SupportSoftPhotosensitive") {
        codes.extend([4, 5]);
    }
    if let Some(current) = current {
        if !codes.contains(&current) {
            codes.push(current);
            codes.sort_unstable();
        }
    }
    codes.into_iter().filter_map(day_night_choice).collect()
}

fn missing(name: &str) -> String {
    format!("{name} answered no data (Ret 607 or a null entry)")
}

pub fn report_of(blocks: &Blocks) -> ImageReport {
    let abilities = &blocks.abilities;
    let param = blocks.param.as_ref().and_then(first);
    let day_code = param.and_then(|entry| int(entry, "DayNightColor"));
    let double_light = ability(abilities, DOUBLE_LIGHT);
    let day_night = match day_code {
        Some(code) => {
            let choice = day_night_choice(code);
            Feature {
                supported: !double_light,
                evidence: if double_light {
                    format!("Camera.Param[0].DayNightColor=0x{code:08X}; {DOUBLE_LIGHT_EVIDENCE}")
                } else {
                    format!(
                        "Camera.Param[0].DayNightColor=0x{code:08X}; choices follow p048d8/E.java:402-405 ({})",
                        ability_evidence(abilities, "SupportSoftPhotosensitive")
                    )
                },
                value: Some(DayNight {
                    code,
                    key: choice
                        .as_ref()
                        .map(|choice| choice.key)
                        .unwrap_or("unknown"),
                    label: choice
                        .as_ref()
                        .map(|choice| choice.label)
                        .unwrap_or("Unknown mode"),
                    options: day_night_options(abilities, Some(code)),
                }),
            }
        }
        None => Feature {
            supported: false,
            evidence: "Camera.Param[0] carries no DayNightColor".to_string(),
            value: None,
        },
    };

    let light_entry = blocks.white_light.as_ref().and_then(first);
    let light_mode = match (double_light, light_entry) {
        (true, Some(entry)) if ability(abilities, "SupportCameraWhiteLight") => {
            let current = entry
                .get("WorkMode")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Feature {
                supported: true,
                evidence: format!("Camera.WhiteLight.WorkMode={current}; {DOUBLE_LIGHT_EVIDENCE}"),
                value: Some(LightMode {
                    label: light_mode_label(&current).unwrap_or("Not one of the app's modes").to_string(),
                    options: light_mode_options(entry, &current),
                    mode: current,
                }),
            }
        }
        (true, _) => Feature {
            supported: false,
            evidence: if ability(abilities, "SupportCameraWhiteLight") {
                missing(WHITE_LIGHT)
            } else {
                ability_evidence(abilities, "SupportCameraWhiteLight")
            },
            value: None,
        },
        (false, _) => Feature {
            supported: false,
            evidence: format!(
                "{}; the app sets the light mode this way only on double-light box cameras (MonitorActivity.java:3115)",
                ability_evidence(abilities, DOUBLE_LIGHT)
            ),
            value: None,
        },
    };

    let scene_value = blocks
        .fish_eye
        .as_ref()
        .and_then(first)
        .and_then(|entry| int(entry, "Secene"));
    let scene = Feature {
        supported: scene_value.is_some(),
        evidence: match scene_value {
            Some(value) => format!("Camera.FishEye.Secene={value}"),
            None => format!(
                "{}; the app's scene picker edits Camera.FishEye.Secene (DevAdvancedSettingActivity.java:641)",
                missing(FISH_EYE)
            ),
        },
        value: scene_value,
    };

    let lux = blocks
        .param_ex
        .as_ref()
        .and_then(first)
        .and_then(|entry| flag(entry, "LowLuxMode"));
    let lux_gate = ability(abilities, "SupportLowLuxMode");
    let low_lux = Feature {
        supported: lux_gate && lux.is_some(),
        evidence: if !lux_gate {
            format!(
                "{}; the app hides the low-lux button without it (MonitorActivity.java:3789)",
                ability_evidence(abilities, "SupportLowLuxMode")
            )
        } else if lux.is_none() {
            "Camera.ParamEx[0] carries no LowLuxMode".to_string()
        } else {
            "Camera.ParamEx[0].LowLuxMode".to_string()
        },
        value: lux,
    };

    let light_gate = ability(abilities, "SupportCameraWhiteLight");
    let light = blocks.white_light.as_ref().and_then(first);
    let white_light = Feature {
        supported: light_gate && light.is_some(),
        evidence: if !light_gate {
            ability_evidence(abilities, "SupportCameraWhiteLight")
        } else if light.is_none() {
            missing(WHITE_LIGHT)
        } else {
            "Camera.WhiteLight WorkPeriod/MoveTrigLight".to_string()
        },
        value: light.map(|entry| LightSchedule {
            mode: entry
                .get("WorkMode")
                .and_then(Value::as_str)
                .map(str::to_string),
            brightness: int(entry, "Brightness"),
            period: entry.get("WorkPeriod").and_then(|period| {
                Some(Period {
                    enable: flag(period, "Enable")?,
                    start: clock(int(period, "SHour")?, int(period, "SMinute")?),
                    end: clock(int(period, "EHour")?, int(period, "EMinute")?),
                })
            }),
            trigger: entry.get("MoveTrigLight").and_then(|trigger| {
                Some(Trigger {
                    level: int(trigger, "Level")?,
                    duration: int(trigger, "Duration")?,
                })
            }),
        }),
    };

    let privacy = gated(
        abilities,
        "SupportOneKeyMaskVideo",
        "G7/c.java:363",
        MASK_VIDEO,
        blocks.mask.as_ref().and_then(first),
        |entry| flag(entry, "Enable"),
    );
    let sleep = gated(
        abilities,
        "SupportTimingSleep",
        "G7/c.java:366",
        TIMING_SLEEP,
        blocks.sleep.as_ref().and_then(first),
        |entry| {
            let period = entry.get("WorkPeriod")?;
            Some(Sleep {
                enable: flag(entry, "Enable")?,
                sleeping: flag(entry, "InSleeping").unwrap_or(false),
                repeat_daily: int(entry, "RepeatType").unwrap_or(0) == 1,
                start: clock(int(period, "SHour")?, int(period, "SMinute")?),
                end: clock(int(period, "EHour")?, int(period, "EMinute")?),
            })
        },
    );
    let extra = blocks.extra.as_ref().and_then(first);
    let status_led = gated(
        abilities,
        "SupportStatusLed",
        "DevBasicSettingActivity.java:342",
        EXTRA_STATE,
        extra,
        |entry| flag(entry, "ison"),
    );
    let voice_tip = gated(
        abilities,
        "SupportCloseVoiceTip",
        "DevBasicSettingActivity.java:349",
        EXTRA_STATE,
        extra,
        |entry| flag(entry, "PlayVoiceTip"),
    );
    let speaker_volume = gated(
        abilities,
        "SupportSetVolume",
        "DevBasicSettingActivity.java:374",
        SPEAKER_VOLUME,
        blocks.speaker.as_ref().and_then(first),
        |entry| volume_of(entry, SPEAKER_VOLUME),
    );
    let mic_volume = match &blocks.mic {
        Some((name, value)) => gated(
            abilities,
            "SupportSetVolume",
            "DevBasicSettingActivity.java:755",
            name,
            first(value),
            |entry| volume_of(entry, name),
        ),
        None => gated(
            abilities,
            "SupportSetVolume",
            "DevBasicSettingActivity.java:755",
            MIC_VOLUME,
            None,
            |entry| volume_of(entry, MIC_VOLUME),
        ),
    };
    ImageReport {
        day_night,
        light_mode,
        scene,
        low_lux,
        white_light,
        privacy,
        sleep,
        status_led,
        voice_tip,
        speaker_volume,
        mic_volume,
    }
}

fn volume_of(entry: &Value, block: &'static str) -> Option<Volume> {
    Some(Volume {
        left: int(entry, "LeftVolume")?,
        right: int(entry, "RightVolume")?,
        block,
    })
}

fn gated<T, F>(
    abilities: &Value,
    key: &str,
    gate_source: &str,
    name: &str,
    entry: Option<&Value>,
    read: F,
) -> Feature<T>
where
    F: Fn(&Value) -> Option<T>,
{
    let allowed = ability(abilities, key);
    let value = entry.and_then(read);
    let evidence = if !allowed {
        format!(
            "{}; the app gates this control on it ({gate_source})",
            ability_evidence(abilities, key)
        )
    } else if value.is_none() {
        missing(name)
    } else {
        format!("{} and {name} answered", ability_evidence(abilities, key))
    };
    Feature {
        supported: allowed && value.is_some(),
        evidence,
        value,
    }
}

pub fn apply_light_mode(entry: &mut Value, raw: &str) -> Result<(), ImageError> {
    let mode = LIGHT_MODES
        .iter()
        .find(|(wire, _)| wire.eq_ignore_ascii_case(raw.trim()))
        .map(|(wire, _)| *wire)
        .ok_or_else(|| {
            ImageError::Invalid(format!(
                "light mode {raw:?} is unknown; use Auto (full colour), Close (general night vision) or Intelligent (double light)"
            ))
        })?;
    if mode == "Intelligent" && !entry.get("MoveTrigLight").is_some_and(Value::is_object) {
        return Err(ImageError::Unsupported {
            feature: "double light",
            evidence: "Camera.WhiteLight carries no MoveTrigLight, and the app offers Double light only with it (p048d8/n.java:150)".to_string(),
        });
    }
    if !entry.get("WorkMode").is_some_and(Value::is_string) {
        return Err(ImageError::Unsupported {
            feature: "light mode",
            evidence: "Camera.WhiteLight carries no WorkMode".to_string(),
        });
    }
    entry["WorkMode"] = json!(mode);
    Ok(())
}

pub fn apply_day_night(entry: &mut Value, code: i64, abilities: &Value) -> Result<(), ImageError> {
    if ability(abilities, DOUBLE_LIGHT) {
        return Err(ImageError::Unsupported {
            feature: "day/night mode",
            evidence: DOUBLE_LIGHT_EVIDENCE.to_string(),
        });
    }
    let Some(choice) = day_night_choice(code) else {
        return Err(ImageError::Invalid(format!(
            "day/night mode {code} is unknown; use 0 auto, 1 colour, 2 black and white, 3 smart alarm light, 4 white lamp auto, 5 IR lamp auto or 6 license plate"
        )));
    };
    if !entry.get("DayNightColor").is_some_and(Value::is_string) {
        return Err(ImageError::Unsupported {
            feature: "day/night mode",
            evidence: "Camera.Param[0] carries no DayNightColor".to_string(),
        });
    }
    let offered = day_night_options(abilities, int(entry, "DayNightColor"));
    if !offered.iter().any(|option| option.code == code) {
        let codes = offered
            .iter()
            .map(|option| option.code.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ImageError::Unsupported {
            feature: "day/night mode",
            evidence: format!(
                "{code} ({}) is not among the modes the app offers this camera ({codes}); 3 and 6 come only from the device's CameraDayLightModes list behind SupportListCameraDayLightModes (p048d8/g.java:421,490), 4 and 5 need SupportSoftPhotosensitive (p048d8/E.java:402-405)",
                choice.label
            ),
        });
    }
    entry["DayNightColor"] = Value::String(format!("0x{code:08X}"));
    Ok(())
}

pub fn apply_scene(entry: &mut Value, scene: i64) -> Result<(), ImageError> {
    if !(0..=255).contains(&scene) {
        return Err(ImageError::Invalid("scene must be 0..255".to_string()));
    }
    if entry.get("Secene").is_none() {
        return Err(ImageError::Unsupported {
            feature: "scene",
            evidence: "Camera.FishEye carries no Secene".to_string(),
        });
    }
    entry["Secene"] = json!(scene);
    Ok(())
}

pub fn apply_low_lux(entry: &mut Value, enable: bool) -> Result<(), ImageError> {
    if entry.get("LowLuxMode").is_none() {
        return Err(ImageError::Unsupported {
            feature: "low-lux mode",
            evidence: "Camera.ParamEx[0] carries no LowLuxMode".to_string(),
        });
    }
    entry["LowLuxMode"] = json!(i64::from(enable));
    Ok(())
}

pub fn apply_light(entry: &mut Value, patch: &LightPatch) -> Result<(), ImageError> {
    if patch.period.is_none()
        && patch.start.is_none()
        && patch.end.is_none()
        && patch.level.is_none()
        && patch.duration.is_none()
    {
        return Err(ImageError::Invalid(
            "name period, start, end, level and/or duration".to_string(),
        ));
    }
    if patch.period.is_some() || patch.start.is_some() || patch.end.is_some() {
        let period = entry
            .get_mut("WorkPeriod")
            .filter(|period| period.is_object())
            .ok_or_else(|| ImageError::Unsupported {
                feature: "white light schedule",
                evidence: "Camera.WhiteLight carries no WorkPeriod".to_string(),
            })?;
        let start = match &patch.start {
            Some(raw) => parse_clock(raw)?,
            None => (
                int(period, "SHour").unwrap_or(0),
                int(period, "SMinute").unwrap_or(0),
            ),
        };
        let end = match &patch.end {
            Some(raw) => parse_clock(raw)?,
            None => (
                int(period, "EHour").unwrap_or(0),
                int(period, "EMinute").unwrap_or(0),
            ),
        };
        if start == end {
            return Err(ImageError::Invalid(
                "the start and end of the light schedule cannot be equal".to_string(),
            ));
        }
        period["SHour"] = json!(start.0);
        period["SMinute"] = json!(start.1);
        period["EHour"] = json!(end.0);
        period["EMinute"] = json!(end.1);
        if let Some(enable) = patch.period {
            period["Enable"] = json!(i64::from(enable));
        }
    }
    if patch.level.is_some() || patch.duration.is_some() {
        let trigger = entry
            .get_mut("MoveTrigLight")
            .filter(|trigger| trigger.is_object())
            .ok_or_else(|| ImageError::Unsupported {
                feature: "light on motion",
                evidence: "Camera.WhiteLight carries no MoveTrigLight".to_string(),
            })?;
        if let Some(level) = patch.level {
            if !LIGHT_LEVELS.contains(&level) {
                return Err(ImageError::Invalid(
                    "level must be 1 (low), 3 (middle) or 5 (high)".to_string(),
                ));
            }
            trigger["Level"] = json!(level);
        }
        if let Some(duration) = patch.duration {
            if !LIGHT_DURATIONS.contains(&duration) {
                return Err(ImageError::Invalid(
                    "duration must be 5, 10, 30, 60, 90 or 120 seconds".to_string(),
                ));
            }
            trigger["Duration"] = json!(duration);
        }
    }
    Ok(())
}

pub fn apply_privacy(entry: &mut Value, enable: bool) -> Result<(), ImageError> {
    entry["Enable"] = json!(enable);
    Ok(())
}

pub fn apply_sleep(entry: &mut Value, patch: &SleepPatch) -> Result<(), ImageError> {
    if patch.enable.is_none()
        && patch.start.is_none()
        && patch.end.is_none()
        && patch.repeat_daily.is_none()
        && !patch.wake
    {
        return Err(ImageError::Invalid(
            "name enable, start, end, repeat and/or wake".to_string(),
        ));
    }
    if patch.wake {
        let sleeping = flag(entry, "InSleeping").unwrap_or(false);
        entry["ManualWakeUp"] = json!(sleeping);
    }
    if patch.start.is_some() || patch.end.is_some() {
        let period = entry
            .get_mut("WorkPeriod")
            .filter(|period| period.is_object())
            .ok_or_else(|| ImageError::Unsupported {
                feature: "timed sleep",
                evidence: "General.TimingSleep carries no WorkPeriod".to_string(),
            })?;
        let start = match &patch.start {
            Some(raw) => parse_clock(raw)?,
            None => (
                int(period, "SHour").unwrap_or(0),
                int(period, "SMinute").unwrap_or(0),
            ),
        };
        let end = match &patch.end {
            Some(raw) => parse_clock(raw)?,
            None => (
                int(period, "EHour").unwrap_or(0),
                int(period, "EMinute").unwrap_or(0),
            ),
        };
        if start == end {
            return Err(ImageError::Invalid(
                "the start and end of the sleep window cannot be equal".to_string(),
            ));
        }
        period["SHour"] = json!(start.0);
        period["SMinute"] = json!(start.1);
        period["EHour"] = json!(end.0);
        period["EMinute"] = json!(end.1);
    }
    if let Some(repeat) = patch.repeat_daily {
        entry["RepeatType"] = json!(i64::from(repeat));
    }
    if let Some(enable) = patch.enable {
        entry["Enable"] = json!(enable);
        entry["ManualWakeUp"] = json!(false);
    }
    Ok(())
}

pub fn apply_indicators(
    entry: &mut Value,
    led: Option<bool>,
    voice_tip: Option<bool>,
) -> Result<(), ImageError> {
    if led.is_none() && voice_tip.is_none() {
        return Err(ImageError::Invalid("name led and/or voice_tip".to_string()));
    }
    if let Some(led) = led {
        entry["ison"] = json!(i64::from(led));
    }
    if let Some(voice) = voice_tip {
        entry["PlayVoiceTip"] = json!(i64::from(voice));
    }
    Ok(())
}

pub fn apply_volume(
    entry: &mut Value,
    left: Option<i64>,
    right: Option<i64>,
) -> Result<(), ImageError> {
    if left.is_none() && right.is_none() {
        return Err(ImageError::Invalid("name left and/or right".to_string()));
    }
    for (key, value) in [("LeftVolume", left), ("RightVolume", right)] {
        if let Some(value) = value {
            if !(0..=100).contains(&value) {
                return Err(ImageError::Invalid("volume must be 0..100".to_string()));
            }
            if entry.get(key).is_none() {
                return Err(ImageError::Unsupported {
                    feature: "volume",
                    evidence: format!("the volume block carries no {key}"),
                });
            }
            entry[key] = json!(value);
        }
    }
    Ok(())
}

async fn within<T, F>(stage: &'static str, window: Duration, call: F) -> Result<T, ImageError>
where
    F: Future<Output = Result<T, DvrError>>,
{
    match tokio::time::timeout(window, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(ImageError::Device { stage, error }),
        Err(_) => Err(ImageError::Timeout { stage, window }),
    }
}

fn stage_read(name: &str) -> &'static str {
    match name {
        CAMERA_PARAM => "Camera.Param read",
        CAMERA_PARAM_EX => "Camera.ParamEx read",
        WHITE_LIGHT => "Camera.WhiteLight read",
        FISH_EYE => "Camera.FishEye read",
        MASK_VIDEO => "General.OneKeyMaskVideo read",
        TIMING_SLEEP => "General.TimingSleep read",
        EXTRA_STATE => "FbExtraStateCtrl read",
        SPEAKER_VOLUME => "fVideo.Volume read",
        MIC_VOLUME => "fVideo.InVolumeIn read",
        MIC_VOLUME_ALIAS => "fVideo.VolumeIn read",
        _ => "config read",
    }
}

fn stage_write(name: &str) -> &'static str {
    match name {
        CAMERA_PARAM => "Camera.Param write",
        CAMERA_PARAM_EX => "Camera.ParamEx write",
        WHITE_LIGHT => "Camera.WhiteLight write",
        FISH_EYE => "Camera.FishEye write",
        MASK_VIDEO => "General.OneKeyMaskVideo write",
        TIMING_SLEEP => "General.TimingSleep write",
        EXTRA_STATE => "FbExtraStateCtrl write",
        SPEAKER_VOLUME => "fVideo.Volume write",
        MIC_VOLUME => "fVideo.InVolumeIn write",
        MIC_VOLUME_ALIAS => "fVideo.VolumeIn write",
        _ => "config write",
    }
}

fn session_field(connection: &Connection) -> String {
    format!("0x{:08X}", connection.session())
}

pub async fn read_block(
    connection: &mut Connection,
    name: &str,
    window: Duration,
) -> Result<Option<Value>, ImageError> {
    let stage = stage_read(name);
    let body = json!({ "Name": name, "SessionID": session_field(connection) });
    let reply = within(stage, window, connection.request(msg::CONFIG_GET, body)).await?;
    let ret = reply.get("Ret").and_then(Value::as_i64).unwrap_or(0);
    match ret {
        100 | 150 => Ok(present(reply.get(name).cloned().unwrap_or(Value::Null))),
        607 => Ok(None),
        other => Err(ImageError::Device {
            stage,
            error: DvrError::Rejected {
                ret: other,
                reason: "the device refused the config read",
            },
        }),
    }
}

async fn write_block(
    connection: &mut Connection,
    name: &str,
    value: Value,
    window: Duration,
) -> Result<(), ImageError> {
    within(
        stage_write(name),
        window,
        connection.config_set(name, value),
    )
    .await
    .map(|_| ())
}

pub async fn read_abilities(
    connection: &mut Connection,
    window: Duration,
) -> Result<Value, ImageError> {
    within("ability read", window, connection.abilities()).await
}

pub async fn read_mic(
    connection: &mut Connection,
    window: Duration,
) -> Result<Option<(&'static str, Value)>, ImageError> {
    for name in [MIC_VOLUME, MIC_VOLUME_ALIAS] {
        if let Some(value) = read_block(connection, name, window).await? {
            return Ok(Some((name, value)));
        }
    }
    Ok(None)
}

pub async fn read_report(
    connection: &mut Connection,
    window: Duration,
) -> Result<ImageReport, ImageError> {
    let abilities = read_abilities(connection, window).await?;
    let mut blocks = Blocks {
        abilities,
        ..Blocks::default()
    };
    blocks.param = read_block(connection, CAMERA_PARAM, window).await?;
    blocks.param_ex = read_block(connection, CAMERA_PARAM_EX, window).await?;
    blocks.white_light = read_block(connection, WHITE_LIGHT, window).await?;
    blocks.fish_eye = read_block(connection, FISH_EYE, window).await?;
    blocks.mask = read_block(connection, MASK_VIDEO, window).await?;
    blocks.sleep = read_block(connection, TIMING_SLEEP, window).await?;
    blocks.extra = read_block(connection, EXTRA_STATE, window).await?;
    blocks.speaker = read_block(connection, SPEAKER_VOLUME, window).await?;
    blocks.mic = read_mic(connection, window).await?;
    Ok(report_of(&blocks))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LiveState {
    pub low_lux: Feature<bool>,
    pub privacy: Feature<bool>,
    pub sleep: Feature<Sleep>,
}

pub fn live_of(blocks: &Blocks) -> LiveState {
    let report = report_of(blocks);
    LiveState {
        low_lux: report.low_lux,
        privacy: report.privacy,
        sleep: report.sleep,
    }
}

pub async fn read_live(
    connection: &mut Connection,
    window: Duration,
) -> Result<LiveState, ImageError> {
    let abilities = read_abilities(connection, window).await?;
    let mut blocks = Blocks::default();
    if ability(&abilities, "SupportLowLuxMode") {
        blocks.param_ex = read_block(connection, CAMERA_PARAM_EX, window).await?;
    }
    if ability(&abilities, "SupportOneKeyMaskVideo") {
        blocks.mask = read_block(connection, MASK_VIDEO, window).await?;
    }
    if ability(&abilities, "SupportTimingSleep") {
        blocks.sleep = read_block(connection, TIMING_SLEEP, window).await?;
    }
    blocks.abilities = abilities;
    Ok(live_of(&blocks))
}

const DEVICE_OWNED: [&str; 2] = ["InSleeping", "ManualWakeUp"];

pub fn kept(before: &Value, requested: &Value, after: &Value) -> bool {
    let (Some(before), Some(requested)) = (first(before), first(requested)) else {
        return first(after) == first(requested);
    };
    let Some(fields) = requested.as_object() else {
        return first(after) == Some(requested);
    };
    let after = first(after);
    fields
        .iter()
        .filter(|(key, _)| !DEVICE_OWNED.contains(&key.as_str()))
        .filter(|(key, value)| before.get(key.as_str()) != Some(*value))
        .all(|(key, value)| after.and_then(|entry| entry.get(key.as_str())) == Some(value))
}

pub struct Edit<'a> {
    pub feature: &'static str,
    pub gates: &'a [&'static str],
    pub names: &'a [&'static str],
}

pub async fn edit<F>(
    connection: &mut Connection,
    window: Duration,
    edit: Edit<'_>,
    mutate: F,
) -> Result<WriteOutcome, ImageError>
where
    F: FnOnce(&Value, &mut Value) -> Result<(), ImageError>,
{
    let abilities = read_abilities(connection, window).await?;
    if let Some(key) = edit.gates.iter().find(|key| !ability(&abilities, key)) {
        return Err(ImageError::Unsupported {
            feature: edit.feature,
            evidence: ability_evidence(&abilities, key),
        });
    }
    let mut found = None;
    for name in edit.names {
        if let Some(value) = read_block(connection, name, window).await? {
            found = Some((*name, value));
            break;
        }
    }
    let (block, before) = found.ok_or_else(|| ImageError::Unsupported {
        feature: edit.feature,
        evidence: missing(edit.names.first().copied().unwrap_or("config")),
    })?;
    let mut requested = before.clone();
    let entry = first_mut(&mut requested).ok_or_else(|| ImageError::Unsupported {
        feature: edit.feature,
        evidence: format!("{block} carries no settings entry"),
    })?;
    mutate(&abilities, entry)?;
    write_block(connection, block, requested.clone(), window).await?;
    let after = read_block(connection, block, window)
        .await?
        .unwrap_or(Value::Null);
    let matches = kept(&before, &requested, &after);
    Ok(WriteOutcome {
        block,
        before,
        requested,
        after,
        matches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvrip::fake::{connect, json_reply, serve, serve_scripted};

    fn abilities() -> Value {
        json!({ "OtherFunction": {
            "SupportCameraWhiteLight": true,
            "SupportCloseVoiceTip": true,
            "SupportSetVolume": true,
            "SupportSetInVolume": true,
            "SupportSoftPhotosensitive": true
        }})
    }

    fn camera_blocks() -> Blocks {
        Blocks {
            abilities: abilities(),
            param: Some(json!([{ "DayNightColor": "0x00000003", "PictureFlip": "0x00000000" }])),
            param_ex: Some(json!([{ "LowLuxMode": 1, "Style": "type1" }])),
            white_light: Some(json!({
                "Brightness": 50,
                "MoveTrigLight": { "Duration": 60, "Level": 3 },
                "WorkMode": "Intelligent",
                "WorkPeriod": { "EHour": 6, "EMinute": 0, "Enable": 1, "SHour": 18, "SMinute": 0 }
            })),
            fish_eye: None,
            mask: None,
            sleep: None,
            extra: Some(json!({ "PlayVoiceTip": 1, "ison": 1 })),
            speaker: Some(json!([{ "AudioMode": "Single", "LeftVolume": 50, "RightVolume": 50 }])),
            mic: Some((
                MIC_VOLUME_ALIAS,
                json!([{ "AnrOpen": true, "AudioMode": "Single", "LeftVolume": 34, "RightVolume": 35 }]),
            )),
        }
    }

    #[test]
    fn the_report_reads_the_captured_camera_blocks() {
        let report = report_of(&camera_blocks());
        let day = report.day_night.value.unwrap();
        assert_eq!((day.code, day.key), (3, "smart_light"));
        assert_eq!(
            day.options
                .iter()
                .map(|choice| choice.code)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5]
        );
        let light = report.white_light.value.unwrap();
        assert_eq!(
            light.period,
            Some(Period {
                enable: true,
                start: "18:00".into(),
                end: "06:00".into()
            })
        );
        assert_eq!(
            light.trigger,
            Some(Trigger {
                level: 3,
                duration: 60
            })
        );
        assert!(report.voice_tip.supported);
        assert_eq!(report.voice_tip.value, Some(true));
        assert_eq!(
            report.mic_volume.value,
            Some(Volume {
                left: 34,
                right: 35,
                block: MIC_VOLUME_ALIAS
            })
        );
        assert_eq!(report.speaker_volume.value.unwrap().left, 50);
    }

    #[test]
    fn controls_without_their_ability_flag_are_gated_with_evidence() {
        let report = report_of(&camera_blocks());
        assert!(!report.low_lux.supported);
        assert_eq!(report.low_lux.value, Some(true));
        assert!(report
            .low_lux
            .evidence
            .contains("SupportLowLuxMode is absent"));
        assert!(!report.status_led.supported);
        assert!(report.status_led.evidence.contains("SupportStatusLed"));
        assert!(!report.privacy.supported);
        assert!(report.privacy.evidence.contains("SupportOneKeyMaskVideo"));
        assert!(!report.sleep.supported);
        assert!(!report.scene.supported);
        assert!(report.scene.evidence.contains("Camera.FishEye"));
    }

    #[test]
    fn a_device_with_the_flags_reports_privacy_and_sleep() {
        let mut blocks = camera_blocks();
        blocks.abilities["OtherFunction"]["SupportOneKeyMaskVideo"] = json!(true);
        blocks.abilities["OtherFunction"]["SupportTimingSleep"] = json!(true);
        blocks.mask = Some(json!({ "Enable": true }));
        blocks.sleep = Some(json!({
            "Enable": false, "InSleeping": false, "ManualWakeUp": false, "RepeatType": 1,
            "WorkPeriod": { "SHour": 23, "SMinute": 30, "EHour": 5, "EMinute": 0 }
        }));
        let report = report_of(&blocks);
        assert_eq!(report.privacy.value, Some(true));
        assert!(report.privacy.supported);
        let sleep = report.sleep.value.unwrap();
        assert_eq!(
            (sleep.start.as_str(), sleep.end.as_str(), sleep.repeat_daily),
            ("23:30", "05:00", true)
        );
    }

    #[tokio::test]
    async fn the_live_read_skips_blocks_the_device_does_not_declare() {
        let replies = vec![json_reply(
            json!({ "Ret": 100, "SystemFunction": abilities() }),
        )];
        let (address, handle) = serve(replies).await;
        let mut connection = connect(&address).await;
        let live = read_live(&mut connection, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(!live.low_lux.supported && !live.privacy.supported && !live.sleep.supported);
        assert_eq!(handle.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_live_read_reports_a_covered_lens() {
        let mut declared = abilities();
        declared["OtherFunction"]["SupportOneKeyMaskVideo"] = json!(true);
        let replies = vec![
            json_reply(json!({ "Ret": 100, "SystemFunction": declared })),
            json_reply(json!({ "Ret": 100, MASK_VIDEO: [{ "Enable": true }] })),
        ];
        let (address, _handle) = serve(replies).await;
        let mut connection = connect(&address).await;
        let live = read_live(&mut connection, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(live.privacy.supported);
        assert_eq!(live.privacy.value, Some(true));
    }

    #[test]
    fn null_entries_count_as_absent() {
        assert_eq!(present(json!([null])), None);
        assert_eq!(present(Value::Null), None);
        assert!(present(json!([{ "Enable": false }])).is_some());
    }

    #[test]
    fn day_night_writes_the_hex_wire_form_and_refuses_unknown_codes() {
        let mut entry = json!({ "DayNightColor": "0x00000003" });
        apply_day_night(&mut entry, 1, &abilities()).unwrap();
        assert_eq!(entry["DayNightColor"], "0x00000001");
        assert!(matches!(
            apply_day_night(&mut entry, 7, &abilities()),
            Err(ImageError::Invalid(_))
        ));
        let mut bare = json!({});
        assert!(matches!(
            apply_day_night(&mut bare, 1, &abilities()),
            Err(ImageError::Unsupported { .. })
        ));
    }

    #[test]
    fn day_night_refuses_modes_the_app_does_not_offer_this_camera() {
        let mut current = json!({ "DayNightColor": "0x00000003" });
        apply_day_night(&mut current, 3, &abilities()).unwrap();
        let mut entry = json!({ "DayNightColor": "0x00000001" });
        for code in [3, 6] {
            let error = apply_day_night(&mut entry, code, &abilities()).unwrap_err();
            assert!(
                matches!(error, ImageError::Unsupported { .. }),
                "{code}: {error}"
            );
        }
        let plain = json!({ "OtherFunction": {} });
        assert!(matches!(
            apply_day_night(&mut entry, 4, &plain),
            Err(ImageError::Unsupported { .. })
        ));
        apply_day_night(&mut entry, 4, &abilities()).unwrap();
        assert_eq!(entry["DayNightColor"], "0x00000004");
    }

    #[test]
    fn kept_ignores_device_owned_status_and_untouched_fields() {
        let before = json!([{ "Enable": false, "InSleeping": true, "ManualWakeUp": false, "RepeatType": 0 }]);
        let requested =
            json!([{ "Enable": false, "InSleeping": true, "ManualWakeUp": true, "RepeatType": 0 }]);
        let after = json!([{ "Enable": false, "InSleeping": false, "ManualWakeUp": false, "RepeatType": 1 }]);
        assert!(kept(&before, &requested, &after));
        let changed =
            json!([{ "Enable": true, "InSleeping": true, "ManualWakeUp": false, "RepeatType": 0 }]);
        assert!(!kept(&before, &changed, &after));
        assert!(kept(&before, &changed, &changed));
    }

    #[test]
    fn the_light_patch_maps_times_levels_and_durations() {
        let mut entry = camera_blocks().white_light.unwrap();
        let patch = LightPatch {
            period: Some(false),
            start: Some("19:15".into()),
            end: Some("05:45".into()),
            level: Some(5),
            duration: Some(30),
        };
        apply_light(&mut entry, &patch).unwrap();
        assert_eq!(
            entry["WorkPeriod"],
            json!({ "EHour": 5, "EMinute": 45, "Enable": 0, "SHour": 19, "SMinute": 15 })
        );
        assert_eq!(
            entry["MoveTrigLight"],
            json!({ "Duration": 30, "Level": 5 })
        );
        assert_eq!(entry["WorkMode"], "Intelligent");
        for bad in [
            LightPatch {
                start: Some("06:00".into()),
                ..LightPatch::default()
            },
            LightPatch {
                level: Some(2),
                ..LightPatch::default()
            },
            LightPatch {
                duration: Some(45),
                ..LightPatch::default()
            },
            LightPatch {
                start: Some("24:00".into()),
                ..LightPatch::default()
            },
            LightPatch::default(),
        ] {
            let mut copy = camera_blocks().white_light.unwrap();
            copy["WorkPeriod"]["SHour"] = json!(18);
            assert!(
                matches!(apply_light(&mut copy, &bad), Err(ImageError::Invalid(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_sleep_patch_clears_manual_wake_up_like_the_app() {
        let mut entry = json!({
            "Enable": false, "InSleeping": false, "ManualWakeUp": true, "RepeatType": 0,
            "WorkPeriod": { "SHour": 0, "SMinute": 0, "EHour": 1, "EMinute": 0 }
        });
        let patch = SleepPatch {
            enable: Some(true),
            start: Some("03:10".into()),
            end: Some("03:20".into()),
            repeat_daily: Some(true),
            wake: false,
        };
        apply_sleep(&mut entry, &patch).unwrap();
        assert_eq!(entry["Enable"], true);
        assert_eq!(entry["ManualWakeUp"], false);
        assert_eq!(entry["RepeatType"], 1);
        assert_eq!(
            entry["WorkPeriod"],
            json!({ "SHour": 3, "SMinute": 10, "EHour": 3, "EMinute": 20 })
        );
    }

    #[test]
    fn indicators_and_volume_write_integers_in_range() {
        let mut extra = json!({ "PlayVoiceTip": 1, "ison": 1 });
        apply_indicators(&mut extra, None, Some(false)).unwrap();
        assert_eq!(extra, json!({ "PlayVoiceTip": 0, "ison": 1 }));
        let mut volume = json!({ "AudioMode": "Single", "LeftVolume": 34, "RightVolume": 35 });
        apply_volume(&mut volume, Some(80), Some(80)).unwrap();
        assert_eq!(volume["LeftVolume"], 80);
        assert_eq!(volume["RightVolume"], 80);
        assert!(matches!(
            apply_volume(&mut volume, Some(101), None),
            Err(ImageError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn edit_writes_the_whole_block_under_its_name_and_reads_it_back() {
        let replies = vec![
            json_reply(json!({ "Ret": 100, "SystemFunction": abilities() })),
            json_reply(
                json!({ "Ret": 100, SPEAKER_VOLUME: [{ "AudioMode": "Single", "LeftVolume": 50, "RightVolume": 50 }] }),
            ),
            json_reply(json!({ "Ret": 100 })),
            json_reply(
                json!({ "Ret": 100, SPEAKER_VOLUME: [{ "AudioMode": "Single", "LeftVolume": 40, "RightVolume": 40 }] }),
            ),
        ];
        let (address, handle) = serve(replies).await;
        let mut connection = connect(&address).await;
        let outcome = edit(
            &mut connection,
            Duration::from_secs(2),
            Edit {
                feature: "speaker volume",
                gates: &["SupportSetVolume"],
                names: &[SPEAKER_VOLUME],
            },
            |_, entry| apply_volume(entry, Some(40), Some(40)),
        )
        .await
        .unwrap();
        assert!(outcome.matches);
        assert_eq!(outcome.block, SPEAKER_VOLUME);
        let seen = handle.await.unwrap();
        let set: Value = serde_json::from_str(seen[2].trim_end_matches(['\0', '\n'])).unwrap();
        assert_eq!(set["Name"], SPEAKER_VOLUME);
        assert_eq!(
            set[SPEAKER_VOLUME],
            json!([{ "AudioMode": "Single", "LeftVolume": 40, "RightVolume": 40 }])
        );
    }

    #[tokio::test]
    async fn edit_refuses_a_write_the_ability_flag_does_not_allow() {
        let replies = vec![json_reply(
            json!({ "Ret": 100, "SystemFunction": abilities() }),
        )];
        let (address, handle) = serve(replies).await;
        let mut connection = connect(&address).await;
        let error = edit(
            &mut connection,
            Duration::from_secs(2),
            Edit {
                feature: "status LED",
                gates: &["SupportStatusLed"],
                names: &[EXTRA_STATE],
            },
            |_, entry| apply_indicators(entry, Some(false), None),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ImageError::Unsupported { .. }), "{error}");
        assert_eq!(handle.await.unwrap().len(), 1);
    }

    #[test]
    fn a_double_light_camera_gets_the_work_mode_picker_instead_of_day_night() {
        let mut blocks = camera_blocks();
        blocks.abilities["OtherFunction"][DOUBLE_LIGHT] = json!(true);
        let report = report_of(&blocks);
        assert!(!report.day_night.supported);
        assert!(report.day_night.evidence.contains("n.java"));
        assert_eq!(report.day_night.value.unwrap().code, 3);
        assert!(report.light_mode.supported);
        let mode = report.light_mode.value.unwrap();
        assert_eq!(
            (mode.mode.as_str(), mode.label.as_str()),
            ("Intelligent", "Double light")
        );
        assert_eq!(
            mode.options
                .iter()
                .map(|choice| choice.mode.as_str())
                .collect::<Vec<_>>(),
            vec!["Auto", "Close", "Intelligent"]
        );
        let plain = report_of(&camera_blocks());
        assert!(plain.day_night.supported);
        assert!(!plain.light_mode.supported);
    }

    #[test]
    fn double_light_needs_a_motion_trigger_and_day_night_is_refused() {
        let mut light = camera_blocks().white_light.unwrap();
        apply_light_mode(&mut light, "close").unwrap();
        assert_eq!(light["WorkMode"], "Close");
        assert_eq!(
            light["MoveTrigLight"],
            json!({ "Duration": 60, "Level": 3 })
        );
        assert!(matches!(
            apply_light_mode(&mut light, "KeepOpen"),
            Err(ImageError::Invalid(_))
        ));
        let mut bare = json!({ "WorkMode": "Auto", "Brightness": 50 });
        assert!(matches!(
            apply_light_mode(&mut bare, "Intelligent"),
            Err(ImageError::Unsupported { .. })
        ));
        let mut blocks = camera_blocks();
        blocks.white_light = Some(bare);
        blocks.abilities["OtherFunction"][DOUBLE_LIGHT] = json!(true);
        let options = report_of(&blocks).light_mode.value.unwrap().options;
        assert_eq!(options.len(), 2);
        let mut double = abilities();
        double["OtherFunction"][DOUBLE_LIGHT] = json!(true);
        let mut entry = json!({ "DayNightColor": "0x00000003" });
        assert!(matches!(
            apply_day_night(&mut entry, 1, &double),
            Err(ImageError::Unsupported { .. })
        ));
        assert_eq!(entry["DayNightColor"], "0x00000003");
    }

    #[tokio::test]
    async fn the_mic_read_falls_back_to_the_block_this_firmware_fills() {
        let (address, _handle) = serve_scripted(|_, _, body| {
            let text = String::from_utf8_lossy(body);
            let reply = if text.contains(MIC_VOLUME_ALIAS) {
                json!({ "Ret": 100, MIC_VOLUME_ALIAS: [{ "LeftVolume": 34, "RightVolume": 35 }] })
            } else {
                json!({ "Ret": 100, MIC_VOLUME: [null] })
            };
            vec![(msg::CONFIG_GET + 1, json_reply(reply))]
        })
        .await;
        let mut connection = connect(&address).await;
        let (name, value) = read_mic(&mut connection, Duration::from_secs(2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(name, MIC_VOLUME_ALIAS);
        assert_eq!(value[0]["LeftVolume"], 34);
    }

    #[tokio::test]
    async fn a_silent_device_times_the_stage_out() {
        let (address, _handle) = serve_scripted(|_, _, _| Vec::new()).await;
        let mut connection = connect(&address).await;
        let error = read_block(&mut connection, SPEAKER_VOLUME, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                ImageError::Timeout {
                    stage: "fVideo.Volume read",
                    ..
                }
            ),
            "{error}"
        );
    }
}
