use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::client::{require_ok, Connection, DvrError};
use super::msg;
use super::settings::{
    backup_file, boolean, compare, describe, first_entry, number, redact, slot, text, EncodeState,
    FieldChange, SettingsError, Stream, VideoFormat, WriteReport, WriteWindow,
};
use super::storage_card::parse_hex;

pub const ENCODE_CAPABILITY: &str = "EncodeCapability";
pub const ENCODE_264_ABILITY: &str = "Encode264ability";
pub const SMART_BLOCK: &str = "AVEnc.SmartH264";
pub const SMART_V2_BLOCK: &str = "AVEnc.SmartH264V2";
pub const LOCATION_BLOCK: &str = "General.Location";
pub const VIDEO_WIDGET_BLOCK: &str = "AVEnc.VideoWidget";
pub const CHANNEL_TITLE: &str = "ChannelTitle";
pub const TITLE_DOT: &str = "TitleDot";
pub const TITLE_DOT_RESERVED: usize = 12;
pub const TITLE_DOT_MAX_WIDTH: usize = 1024;
pub const TITLE_DOT_MAX_HEIGHT: usize = 64;
pub const PAL_FPS: i64 = 25;
pub const NTSC_FPS: i64 = 30;

pub const RESOLUTIONS: [(&str, u64); 32] = [
    ("D1", 405_504),
    ("HD1", 202_752),
    ("BCIF", 202_752),
    ("CIF", 101_376),
    ("QCIF", 25_344),
    ("VGA", 307_200),
    ("QVGA", 76_800),
    ("SVCD", 230_400),
    ("QQVGA", 20_480),
    ("ND1", 46_080),
    ("960H", 534_528),
    ("720P", 921_600),
    ("1_3M", 1_228_800),
    ("UXGA", 1_920_000),
    ("1080P", 2_073_600),
    ("WUXGA", 2_304_000),
    ("2_5M", 2_635_776),
    ("3M", 3_145_728),
    ("5M", 4_915_200),
    ("1080N", 1_019_520),
    ("4M", 3_686_400),
    ("6M", 6_291_456),
    ("8M", 7_990_272),
    ("12M", 12_000_000),
    ("4K", 8_294_400),
    ("720N", 460_800),
    ("WSVGA", 589_824),
    ("NHD", 230_400),
    ("3M_N", 1_572_864),
    ("4M_N", 1_843_200),
    ("5M_N", 2_457_600),
    ("4K_N", 4_147_200),
];

pub fn resolution_pixels(token: &str) -> Option<u64> {
    let trimmed = token.trim();
    if let Some((_, pixels)) = RESOLUTIONS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(trimmed))
    {
        return Some(*pixels);
    }
    let (width, height) = trimmed
        .split_once('*')
        .or_else(|| trimmed.split_once(['x', 'X']))?;
    let width: u64 = width.trim().parse().ok()?;
    let height: u64 = height.trim().parse().ok()?;
    (width > 0 && height > 0).then_some(width * height)
}

pub fn mask_resolutions(mask: u64) -> Vec<String> {
    RESOLUTIONS
        .iter()
        .enumerate()
        .filter(|(bit, _)| mask & (1u64 << bit) != 0)
        .map(|(_, (name, _))| name.to_string())
        .collect()
}

pub fn resolution_index(token: &str) -> Option<usize> {
    RESOLUTIONS
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case(token.trim()))
}

fn hex_at(value: &Value, path: &str) -> u64 {
    match super::settings::at(value, path) {
        Some(Value::String(raw)) => parse_hex(raw),
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        _ => 0,
    }
}

pub fn max_fps(budget: i64, pixels: u64, ceiling: i64) -> i64 {
    if pixels == 0 {
        return ceiling;
    }
    let mut fps = ceiling;
    while fps > 0 && (pixels as i64).saturating_mul(fps) > budget {
        fps -= 1;
    }
    fps
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct StreamLimits {
    pub resolutions: Vec<String>,
    pub max_fps: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EncodeCapabilityView {
    pub max_bitrate: Option<i64>,
    pub max_power: Option<i64>,
    pub fps_ceiling: i64,
    pub main: StreamLimits,
    pub extra: StreamLimits,
    pub block: Value,
}

impl EncodeCapabilityView {
    pub fn stream(&self, stream: Stream) -> &StreamLimits {
        match stream {
            Stream::Main => &self.main,
            Stream::Extra => &self.extra,
        }
    }
}

fn consumption(format: &VideoFormat) -> i64 {
    let pixels = format
        .resolution
        .as_deref()
        .and_then(resolution_pixels)
        .unwrap_or(0);
    (pixels as i64).saturating_mul(format.fps.unwrap_or(0).max(0))
}

fn limits_for(mask: u64, current: Option<&str>, budget: Option<i64>, ceiling: i64) -> StreamLimits {
    let mut resolutions = mask_resolutions(mask);
    if let Some(budget) = budget {
        resolutions.retain(|name| {
            resolution_pixels(name)
                .map(|pixels| pixels as i64 <= budget)
                .unwrap_or(true)
        });
    }
    if let Some(current) = current.map(str::trim).filter(|token| !token.is_empty()) {
        if !resolutions
            .iter()
            .any(|name| name.eq_ignore_ascii_case(current))
        {
            resolutions.insert(0, current.to_string());
        }
    }
    let max_fps = resolutions
        .iter()
        .map(|name| {
            let fps = match (budget, resolution_pixels(name)) {
                (Some(budget), Some(pixels)) => max_fps(budget, pixels, ceiling),
                _ => ceiling,
            };
            (name.clone(), fps)
        })
        .collect();
    StreamLimits {
        resolutions,
        max_fps,
    }
}

pub fn capability_view(
    capability: &Value,
    encode: &EncodeState,
    fps_ceiling: i64,
) -> EncodeCapabilityView {
    let (entry, _) = first_entry(capability);
    let power = hex_at(entry, "MaxEncodePowerPerChannel.0");
    let max_power = (power > 0).then_some(power as i64);
    let main_mask = match hex_at(entry, "ImageSizePerChannel.0") {
        0 => hex_at(entry, "EncodeInfo.0.ResolutionMask"),
        mask => mask,
    };
    let main_index = encode.main.resolution.as_deref().and_then(resolution_index);
    let extra_mask = main_index
        .map(|index| hex_at(entry, &format!("ExImageSizePerChannelEx.0.{index}")))
        .filter(|mask| *mask != 0)
        .unwrap_or_else(|| hex_at(entry, "CombEncodeInfo.0.ResolutionMask"));
    let main_budget = max_power.map(|power| power - consumption(&encode.extra));
    let extra_budget = max_power.map(|power| power - consumption(&encode.main));
    EncodeCapabilityView {
        max_bitrate: number(entry, "MaxBitrate").filter(|value| *value > 0),
        max_power,
        fps_ceiling,
        main: limits_for(
            main_mask,
            encode.main.resolution.as_deref(),
            main_budget,
            fps_ceiling,
        ),
        extra: limits_for(
            extra_mask,
            encode.extra.resolution.as_deref(),
            extra_budget,
            fps_ceiling,
        ),
        block: redact(capability),
    }
}

pub fn fps_ceiling(location: &Value) -> i64 {
    let (entry, _) = first_entry(location);
    match text(entry, "VideoFormat").as_deref().map(str::trim) {
        Some(format) if format.eq_ignore_ascii_case("NTSC") => NTSC_FPS,
        _ => PAL_FPS,
    }
}

pub fn check_encode(
    view: &EncodeCapabilityView,
    stream: Stream,
    current: &VideoFormat,
    fps: Option<i64>,
    resolution: Option<&str>,
) -> Result<(), SettingsError> {
    let limits = view.stream(stream);
    if let Some(token) = resolution.map(str::trim) {
        if !limits.resolutions.is_empty()
            && !limits
                .resolutions
                .iter()
                .any(|name| name.eq_ignore_ascii_case(token))
        {
            return Err(SettingsError::Invalid {
                field: "resolution",
                detail: format!(
                    "{token} is not in the {} stream EncodeCapability list ({})",
                    stream.label(),
                    limits.resolutions.join(", ")
                ),
            });
        }
    }
    if let Some(fps) = fps {
        let target = resolution
            .map(str::trim)
            .map(str::to_string)
            .or_else(|| current.resolution.clone());
        let ceiling = target
            .as_deref()
            .and_then(|token| {
                limits
                    .max_fps
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(token))
                    .map(|(_, fps)| *fps)
            })
            .unwrap_or(view.fps_ceiling);
        if fps > ceiling {
            return Err(SettingsError::Invalid {
                field: "fps",
                detail: format!(
                    "{fps} fps is above {ceiling}, the EncodeCapability ceiling for the {} stream at {}",
                    stream.label(),
                    target.unwrap_or_else(|| "this resolution".to_string())
                ),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmartMode {
    Off,
    Smart,
    SmartPlus,
}

impl SmartMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Some(SmartMode::Off),
            "smart" | "on" => Some(SmartMode::Smart),
            "smart_plus" | "plus" => Some(SmartMode::SmartPlus),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SmartMode::Off => "off",
            SmartMode::Smart => "smart",
            SmartMode::SmartPlus => "smart_plus",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct SmartAbility {
    pub smart: bool,
    pub smart_plus: bool,
}

pub fn smart_ability(reply: &Value) -> Option<SmartAbility> {
    let block = reply.get(ENCODE_264_ABILITY)?;
    let (entry, _) = first_entry(block);
    let flag = |key: &str| hex_at(entry, &format!("{key}.0")) > 0;
    Some(SmartAbility {
        smart: flag("uIntel264"),
        smart_plus: flag("uIntel264Plus"),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct SmartEncodeState {
    pub mode: SmartMode,
    pub modes: Vec<SmartMode>,
    pub source: &'static str,
    pub ability: Option<SmartAbility>,
    pub ability_block: Value,
    pub block: Value,
}

pub fn smart_v2_mode(block: &Value, ability: SmartAbility) -> SmartMode {
    let (entry, _) = first_entry(block);
    let plus = number_at(entry, "Smart264PlusV2.0.SmartH264Plus").unwrap_or(0);
    let smart = super::settings::at(entry, "Smart264V2.0.SmartH264")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if plus > 0 && ability.smart_plus {
        SmartMode::SmartPlus
    } else if smart && ability.smart {
        SmartMode::Smart
    } else {
        SmartMode::Off
    }
}

fn number_at(value: &Value, path: &str) -> Option<i64> {
    super::settings::at(value, path).and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_bool().map(i64::from))
            .or_else(|| item.as_str().map(|raw| parse_hex(raw) as i64))
    })
}

pub fn smart_v2_patch(
    before: &Value,
    mode: SmartMode,
) -> Result<(Value, Vec<String>), SettingsError> {
    let mut desired = before.clone();
    let (_, prefix) = first_entry(before);
    let plus_key = format!("{prefix}Smart264PlusV2.0.SmartH264Plus");
    let smart_key = format!("{prefix}Smart264V2.0.SmartH264");
    let (plus, smart) = match mode {
        SmartMode::Off => (0, false),
        SmartMode::Smart => (0, true),
        SmartMode::SmartPlus => (1, true),
    };
    for (key, value) in [(&plus_key, json!(plus)), (&smart_key, json!(smart))] {
        let entry = slot(&mut desired, key).ok_or_else(|| SettingsError::Missing {
            block: SMART_V2_BLOCK,
            path: key.clone(),
        })?;
        *entry = value;
    }
    Ok((desired, vec![plus_key, smart_key]))
}

#[derive(Debug, Clone, Serialize)]
pub struct OsdState {
    pub channel: usize,
    pub name: Option<String>,
    pub titles: Vec<String>,
    pub title_shown: Option<bool>,
    pub time_shown: Option<bool>,
    pub dot_required: bool,
    pub block: Value,
}

#[derive(Debug, Clone, Default)]
pub struct OsdPatch {
    pub channel: usize,
    pub name: Option<String>,
    pub title_shown: Option<bool>,
    pub time_shown: Option<bool>,
}

pub fn check_title(name: &str) -> Result<String, SettingsError> {
    let trimmed = name.trim();
    let limit = if trimmed.is_ascii() { 32 } else { 21 };
    if trimmed.is_empty() || trimmed.chars().count() > limit {
        return Err(SettingsError::Invalid {
            field: "name",
            detail: format!(
                "the title must hold 1..{limit} characters ({limit} is the mobile app limit for this alphabet)"
            ),
        });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(SettingsError::Invalid {
            field: "name",
            detail: "the title may not hold control characters".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

fn widget_entry(block: &Value, channel: usize) -> (&Value, String) {
    match block {
        Value::Array(items) => (
            items.get(channel).unwrap_or(&Value::Null),
            format!("{channel}."),
        ),
        _ => (block, String::new()),
    }
}

pub fn widget_patch(
    before: &Value,
    patch: &OsdPatch,
) -> Result<(Value, Vec<String>), SettingsError> {
    let mut desired = before.clone();
    let (_, prefix) = widget_entry(before, patch.channel);
    let mut fields = Vec::new();
    let mut edits: Vec<(String, Value)> = Vec::new();
    if let Some(name) = patch.name.as_deref() {
        edits.push((format!("{prefix}ChannelTitle.Name"), json!(name)));
    }
    if let Some(shown) = patch.title_shown {
        edits.push((
            format!("{prefix}ChannelTitleAttribute.EncodeBlend"),
            json!(shown),
        ));
    }
    if let Some(shown) = patch.time_shown {
        edits.push((
            format!("{prefix}TimeTitleAttribute.EncodeBlend"),
            json!(shown),
        ));
    }
    for (key, value) in edits {
        let entry = slot(&mut desired, &key).ok_or_else(|| SettingsError::Missing {
            block: VIDEO_WIDGET_BLOCK,
            path: key.clone(),
        })?;
        *entry = value;
        fields.push(key);
    }
    Ok((desired, fields))
}

pub fn title_dot_frame(width: usize, height: usize, bits: &[u8]) -> Result<Vec<u8>, SettingsError> {
    if width == 0 || width % 8 != 0 || width > TITLE_DOT_MAX_WIDTH {
        return Err(SettingsError::Invalid {
            field: "width",
            detail: format!("{width} must be a positive multiple of 8 up to {TITLE_DOT_MAX_WIDTH}"),
        });
    }
    if height == 0 || height > TITLE_DOT_MAX_HEIGHT {
        return Err(SettingsError::Invalid {
            field: "height",
            detail: format!("{height} must be 1..{TITLE_DOT_MAX_HEIGHT}"),
        });
    }
    let expected = width * height / 8;
    if bits.len() != expected {
        return Err(SettingsError::Invalid {
            field: "bits",
            detail: format!(
                "{} bytes do not match {width}x{height} at one bit per pixel ({expected} bytes)",
                bits.len()
            ),
        });
    }
    let mut frame = Vec::with_capacity(4 + TITLE_DOT_RESERVED + expected);
    frame.extend_from_slice(&(width as u16).to_le_bytes());
    frame.extend_from_slice(&(height as u16).to_le_bytes());
    frame.extend_from_slice(&[0u8; TITLE_DOT_RESERVED]);
    frame.extend_from_slice(bits);
    Ok(frame)
}

fn slow(block: &'static str, detail: &str) -> SettingsError {
    SettingsError::Slow {
        block,
        detail: detail.to_string(),
    }
}

impl Connection {
    pub async fn encode_capability_raw(&mut self) -> Value {
        let body =
            json!({ "Name": ENCODE_CAPABILITY, "SessionID": format!("0x{:08X}", self.session()) });
        match self.request(msg::ABILITY, body).await {
            Ok(reply) if require_ok(&reply).is_ok() => {
                reply.get(ENCODE_CAPABILITY).cloned().unwrap_or(Value::Null)
            }
            _ => Value::Null,
        }
    }

    pub async fn encode_capability_view(&mut self, encode: &EncodeState) -> EncodeCapabilityView {
        let capability = self.encode_capability_raw().await;
        let ceiling = match self.setting_block(LOCATION_BLOCK).await {
            Ok(location) => fps_ceiling(&location),
            Err(_) => PAL_FPS,
        };
        capability_view(&capability, encode, ceiling)
    }

    async fn smart_ability_read(&mut self) -> Option<(SmartAbility, Value)> {
        let body =
            json!({ "Name": ENCODE_264_ABILITY, "SessionID": format!("0x{:08X}", self.session()) });
        let reply = self.request(msg::ABILITY, body).await.ok()?;
        require_ok(&reply).ok()?;
        let ability = smart_ability(&reply)?;
        Some((
            ability,
            reply
                .get(ENCODE_264_ABILITY)
                .cloned()
                .unwrap_or(Value::Null),
        ))
    }

    pub async fn smart_encode_state(&mut self) -> Result<Option<SmartEncodeState>, SettingsError> {
        if let Ok(block) = self.setting_block(SMART_V2_BLOCK).await {
            if let Some((ability, ability_block)) = self.smart_ability_read().await {
                let mut modes = vec![SmartMode::Off];
                if ability.smart {
                    modes.push(SmartMode::Smart);
                }
                if ability.smart_plus {
                    modes.push(SmartMode::SmartPlus);
                }
                return Ok(Some(SmartEncodeState {
                    mode: smart_v2_mode(&block, ability),
                    modes,
                    source: SMART_V2_BLOCK,
                    ability: Some(ability),
                    ability_block,
                    block: redact(&block),
                }));
            }
        }
        match self.setting_block(SMART_BLOCK).await {
            Ok(block) => {
                let (entry, _) = first_entry(&block);
                let on = boolean(entry, "SmartH264").unwrap_or(false);
                Ok(Some(SmartEncodeState {
                    mode: if on { SmartMode::Smart } else { SmartMode::Off },
                    modes: vec![SmartMode::Off, SmartMode::Smart],
                    source: SMART_BLOCK,
                    ability: None,
                    ability_block: Value::Null,
                    block: redact(&block),
                }))
            }
            Err(SettingsError::Device(DvrError::Rejected { .. }))
            | Err(SettingsError::Missing { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn write_smart_encode(
        &mut self,
        mode: SmartMode,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let state = match tokio::time::timeout(write.window(), self.smart_encode_state()).await {
            Ok(result) => result?,
            Err(_) => {
                self.discard_late_replies().await;
                return Err(slow(
                    SMART_V2_BLOCK,
                    "the smart encode read before the write never came",
                ));
            }
        };
        let state = state.ok_or_else(|| SettingsError::Missing {
            block: SMART_V2_BLOCK,
            path: format!("{SMART_V2_BLOCK}, {ENCODE_264_ABILITY} and {SMART_BLOCK}"),
        })?;
        if !state.modes.contains(&mode) {
            return Err(SettingsError::Invalid {
                field: "smart",
                detail: format!(
                    "{} is not offered by this device ({} reports {})",
                    mode.label(),
                    ENCODE_264_ABILITY,
                    state
                        .modes
                        .iter()
                        .map(|mode| mode.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        if state.source == SMART_V2_BLOCK {
            let before = self
                .setting_block_within(SMART_V2_BLOCK, write.window())
                .await?;
            let (desired, fields) = smart_v2_patch(&before, mode)?;
            return self
                .write_setting(SMART_V2_BLOCK, desired, &fields, write)
                .await;
        }
        let before = self
            .setting_block_within(SMART_BLOCK, write.window())
            .await?;
        let mut desired = before.clone();
        let (_, prefix) = first_entry(&before);
        let key = format!("{prefix}SmartH264");
        let entry = slot(&mut desired, &key).ok_or_else(|| SettingsError::Missing {
            block: SMART_BLOCK,
            path: key.clone(),
        })?;
        *entry = json!(mode != SmartMode::Off);
        self.write_setting(SMART_BLOCK, desired, &[key], write)
            .await
    }

    pub async fn channel_titles(&mut self) -> Result<Vec<String>, SettingsError> {
        let body =
            json!({ "Name": CHANNEL_TITLE, "SessionID": format!("0x{:08X}", self.session()) });
        let reply = self.request(msg::CHANNEL_TITLE_GET, body).await?;
        require_ok(&reply)?;
        Ok(reply
            .get(CHANNEL_TITLE)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn put_channel_titles(&mut self, titles: &[String]) -> Result<(), SettingsError> {
        let body = json!({
            "Name": CHANNEL_TITLE,
            CHANNEL_TITLE: titles,
            "SessionID": format!("0x{:08X}", self.session()),
        });
        let reply = self.request(msg::CHANNEL_TITLE_SET, body).await?;
        require_ok(&reply)?;
        Ok(())
    }

    async fn commit_titles(
        &mut self,
        titles: &[String],
        window: Duration,
    ) -> Result<Value, SettingsError> {
        tokio::time::timeout(window, self.put_channel_titles(titles))
            .await
            .map_err(|_| {
                slow(
                    CHANNEL_TITLE,
                    "the title write was sent and the device never acknowledged it",
                )
            })??;
        let read = tokio::time::timeout(window, self.channel_titles())
            .await
            .map_err(|_| {
                slow(
                    CHANNEL_TITLE,
                    "the title read-back after the write never came",
                )
            })??;
        Ok(json!(read))
    }

    pub async fn osd_state(&mut self, channel: usize) -> Result<OsdState, SettingsError> {
        let block = self.setting_block(VIDEO_WIDGET_BLOCK).await?;
        let titles = self.channel_titles().await.unwrap_or_default();
        let (entry, _) = widget_entry(&block, channel);
        let attribute = |key: &str| {
            entry
                .get(key)
                .and_then(|value| boolean(value, "EncodeBlend"))
        };
        Ok(OsdState {
            channel,
            name: entry
                .get("ChannelTitle")
                .and_then(|title| text(title, "Name")),
            dot_required: titles.len() <= 1,
            titles,
            title_shown: attribute("ChannelTitleAttribute"),
            time_shown: attribute("TimeTitleAttribute"),
            block: redact(&block),
        })
    }

    pub async fn osd_state_within(
        &mut self,
        channel: usize,
        window: Duration,
    ) -> Result<OsdState, SettingsError> {
        match tokio::time::timeout(window, self.osd_state(channel)).await {
            Ok(result) => result,
            Err(_) => {
                self.discard_late_replies().await;
                Err(slow(VIDEO_WIDGET_BLOCK, "the read never came"))
            }
        }
    }

    pub async fn write_video_widget(
        &mut self,
        patch: &OsdPatch,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let before = self
            .setting_block_within(VIDEO_WIDGET_BLOCK, write.window())
            .await?;
        let (desired, fields) = widget_patch(&before, patch)?;
        if fields.is_empty() {
            return Err(SettingsError::Invalid {
                field: "osd",
                detail: "name at least one of name, title or time".to_string(),
            });
        }
        self.write_setting(VIDEO_WIDGET_BLOCK, desired, &fields, write)
            .await
    }

    pub async fn write_channel_title(
        &mut self,
        channel: usize,
        name: &str,
        write: &WriteWindow,
    ) -> Result<WriteReport, SettingsError> {
        let before_titles = tokio::time::timeout(write.window(), self.channel_titles())
            .await
            .map_err(|_| slow(CHANNEL_TITLE, "the title read before the write never came"))??;
        if channel >= before_titles.len() {
            return Err(SettingsError::Missing {
                block: CHANNEL_TITLE,
                path: format!("{CHANNEL_TITLE}.{channel}"),
            });
        }
        let before = json!(before_titles);
        let backup = backup_file(CHANNEL_TITLE, &before, write.dir())?;
        let mut wanted = before_titles.clone();
        wanted[channel] = name.to_string();
        let desired = json!(wanted);
        let fields = vec![channel.to_string()];
        let report =
            |fields: Vec<FieldChange>, read_back: &Value, reverted, restored| WriteReport {
                block: CHANNEL_TITLE.to_string(),
                backup: backup.display().to_string(),
                fields,
                written: desired.clone(),
                read_back: read_back.clone(),
                reverted,
                restored,
            };
        let read_back = match self.commit_titles(&wanted, write.window()).await {
            Ok(value) => value,
            Err(error) => {
                self.discard_late_replies().await;
                return Err(
                    match self.commit_titles(&before_titles, write.window()).await {
                        Ok(_) => SettingsError::WriteFailed {
                            block: CHANNEL_TITLE.to_string(),
                            backup: backup.display().to_string(),
                            detail: error.to_string(),
                        },
                        Err(revert) => SettingsError::RevertFailed {
                            block: CHANNEL_TITLE.to_string(),
                            backup: backup.display().to_string(),
                            detail: format!("{error}; the revert then failed: {revert}"),
                        },
                    },
                );
            }
        };
        let compared = compare(&fields, &before, &desired, &read_back);
        if compared.iter().all(|field| field.matches) {
            return Ok(report(compared, &read_back, false, Vec::new()));
        }
        let restored_block = self
            .commit_titles(&before_titles, write.window())
            .await
            .map_err(|error| SettingsError::RevertFailed {
                block: CHANNEL_TITLE.to_string(),
                backup: backup.display().to_string(),
                detail: error.to_string(),
            })?;
        let restored = compare(&fields, &before, &before, &restored_block);
        let drift = describe(&restored);
        if !drift.is_empty() {
            return Err(SettingsError::RevertFailed {
                block: CHANNEL_TITLE.to_string(),
                backup: backup.display().to_string(),
                detail: format!("the copy was written back but {drift}"),
            });
        }
        Ok(report(compared, &read_back, true, restored))
    }

    pub async fn write_title_dot(
        &mut self,
        width: usize,
        height: usize,
        bits: &[u8],
        window: Duration,
    ) -> Result<i64, SettingsError> {
        let frame = title_dot_frame(width, height, bits)?;
        let reply = tokio::time::timeout(window, self.request_bytes(msg::TITLE_DOT_SET, &frame))
            .await
            .map_err(|_| {
                slow(
                    TITLE_DOT,
                    "the title bitmap was sent and the device never acknowledged it",
                )
            })??;
        Ok(require_ok(&reply)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(main: (&str, i64), extra: (&str, i64)) -> EncodeState {
        EncodeState::from_block(&json!({
            "MainFormat": { "Video": { "Resolution": main.0, "FPS": main.1 } },
            "ExtraFormat": { "Video": { "Resolution": extra.0, "FPS": extra.1 } },
        }))
    }

    #[test]
    fn resolutions_resolve_by_sdk_name_or_width_times_height() {
        assert_eq!(resolution_pixels("6M"), Some(6_291_456));
        assert_eq!(resolution_pixels("1080p"), Some(2_073_600));
        assert_eq!(resolution_pixels("3072*2048"), Some(6_291_456));
        assert_eq!(resolution_pixels("640x360"), Some(230_400));
        assert_eq!(resolution_pixels("bogus"), None);
        assert_eq!(mask_resolutions(0x0020_4000), vec!["1080P", "6M"]);
    }

    #[test]
    fn the_fps_ceiling_subtracts_what_the_other_stream_spends() {
        let capability = json!({
            "MaxEncodePowerPerChannel": ["0x07800000"],
            "ImageSizePerChannel": ["0x00204000"],
            "ExImageSizePerChannelEx": [["0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0", "0x0000000A"]],
        });
        let view = capability_view(&capability, &state(("6M", 12), ("HD1", 12)), PAL_FPS);
        assert_eq!(view.main.resolutions, vec!["1080P", "6M"]);
        assert_eq!(view.main.max_fps["6M"], 19);
        assert_eq!(view.main.max_fps["1080P"], 25);
        assert_eq!(view.extra.resolutions, vec!["HD1", "CIF"]);
        let current = &state(("6M", 12), ("HD1", 12)).main;
        assert!(check_encode(&view, Stream::Main, current, Some(19), None).is_ok());
        assert!(check_encode(&view, Stream::Main, current, Some(20), None).is_err());
        assert!(check_encode(&view, Stream::Main, current, Some(25), Some("1080P")).is_ok());
        assert!(check_encode(&view, Stream::Main, current, None, Some("4K")).is_err());
    }

    #[test]
    fn without_a_capability_the_form_keeps_the_current_token_and_the_video_standard() {
        let view = capability_view(
            &Value::Null,
            &state(("3072*2048", 12), ("640*360", 12)),
            NTSC_FPS,
        );
        assert_eq!(view.main.resolutions, vec!["3072*2048"]);
        assert_eq!(view.main.max_fps["3072*2048"], NTSC_FPS);
        assert_eq!(fps_ceiling(&json!({ "VideoFormat": "NTSC" })), NTSC_FPS);
        assert_eq!(fps_ceiling(&json!({ "VideoFormat": "PAL" })), PAL_FPS);
    }

    #[test]
    fn smart_modes_follow_the_v2_flags_and_the_ability() {
        let both = SmartAbility {
            smart: true,
            smart_plus: true,
        };
        let block = |plus: i64, smart: bool| json!([{ "Smart264PlusV2": [{ "SmartH264Plus": plus }], "Smart264V2": [{ "SmartH264": smart }] }]);
        assert_eq!(smart_v2_mode(&block(1, true), both), SmartMode::SmartPlus);
        assert_eq!(smart_v2_mode(&block(0, true), both), SmartMode::Smart);
        assert_eq!(
            smart_v2_mode(
                &block(1, true),
                SmartAbility {
                    smart: false,
                    smart_plus: false
                }
            ),
            SmartMode::Off
        );
        let (patched, fields) = smart_v2_patch(&block(0, false), SmartMode::SmartPlus).unwrap();
        assert_eq!(patched, block(1, true));
        assert_eq!(
            fields,
            vec![
                "0.Smart264PlusV2.0.SmartH264Plus",
                "0.Smart264V2.0.SmartH264"
            ]
        );
        assert_eq!(
            smart_ability(
                &json!({ "Encode264ability": { "uIntel264": ["0x00000000", "0x1"], "uIntel264Plus": ["0x00000001"] } })
            ),
            Some(SmartAbility {
                smart: false,
                smart_plus: true
            })
        );
    }

    #[test]
    fn the_widget_patch_reaches_the_requested_channel_only() {
        let block = json!([
            { "ChannelTitle": { "Name": "a" }, "ChannelTitleAttribute": { "EncodeBlend": true }, "TimeTitleAttribute": { "EncodeBlend": true } },
            { "ChannelTitle": { "Name": "b" }, "ChannelTitleAttribute": { "EncodeBlend": true }, "TimeTitleAttribute": { "EncodeBlend": true } }
        ]);
        let patch = OsdPatch {
            channel: 1,
            name: Some("c".to_string()),
            title_shown: None,
            time_shown: Some(false),
        };
        let (desired, fields) = widget_patch(&block, &patch).unwrap();
        assert_eq!(desired[0], block[0]);
        assert_eq!(desired[1]["ChannelTitle"]["Name"], "c");
        assert_eq!(desired[1]["TimeTitleAttribute"]["EncodeBlend"], false);
        assert_eq!(
            fields,
            vec!["1.ChannelTitle.Name", "1.TimeTitleAttribute.EncodeBlend"]
        );
    }

    #[test]
    fn titles_follow_the_app_length_limits() {
        assert_eq!(check_title("  Gate  ").unwrap(), "Gate");
        assert!(check_title(&"a".repeat(32)).is_ok());
        assert!(check_title(&"a".repeat(33)).is_err());
        assert!(check_title(&"é".repeat(21)).is_ok());
        assert!(check_title(&"é".repeat(22)).is_err());
        assert!(check_title("   ").is_err());
        assert!(check_title(
            "a
b"
        )
        .is_err());
    }

    #[test]
    fn the_title_dot_frame_is_little_endian_size_then_reserved_then_bits() {
        let bits = vec![0xAAu8; 2 * 3];
        let frame = title_dot_frame(16, 3, &bits).unwrap();
        assert_eq!(&frame[..4], &[16, 0, 3, 0]);
        assert_eq!(&frame[4..16], &[0u8; 12]);
        assert_eq!(&frame[16..], bits.as_slice());
        assert!(title_dot_frame(12, 3, &bits).is_err());
        assert!(title_dot_frame(16, 3, &bits[..5]).is_err());
        assert!(title_dot_frame(16, 0, &[]).is_err());
    }
}
