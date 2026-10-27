use serde::Serialize;
use serde_json::{json, Value};

use super::client::{require_ok, Connection, DvrError};
use super::msg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteLightMode {
    Off,
    Intelligent,
    Always,
}

impl WhiteLightMode {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "off" => WhiteLightMode::Off,
            "intelligent" | "auto" => WhiteLightMode::Intelligent,
            "always" | "on" => WhiteLightMode::Always,
            _ => return None,
        })
    }

    pub fn wire(self) -> &'static str {
        match self {
            WhiteLightMode::Off => "Off",
            WhiteLightMode::Intelligent => "Intelligent",
            WhiteLightMode::Always => "Always",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordMode {
    Config,
    Manual,
    Off,
}

impl RecordMode {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "config" | "schedule" => RecordMode::Config,
            "manual" => RecordMode::Manual,
            "off" | "stop" => RecordMode::Off,
            _ => return None,
        })
    }

    pub fn wire(self) -> &'static str {
        match self {
            RecordMode::Config => "ConfigRecord",
            RecordMode::Manual => "ManualRecord",
            RecordMode::Off => "Off",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Capability {
    pub supported: bool,
    pub evidence: String,
}

impl Capability {
    fn new(supported: bool, evidence: impl Into<String>) -> Self {
        Self {
            supported,
            evidence: evidence.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceCapabilities {
    pub device_type: Option<String>,
    pub white_light: Capability,
    pub picture_flip: Capability,
    pub snapshot: Capability,
    pub record_mode: Capability,
    pub talk: Capability,
    pub ptz_tour: Capability,
    pub ptz_locate: Capability,
    pub ptz_direction: Capability,
    pub tour_lines: Vec<TourLine>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TourPoint {
    pub preset: i64,
    pub seconds: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TourLine {
    pub channel: usize,
    pub id: i64,
    pub name: String,
    pub points: Vec<TourPoint>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PresetEntry {
    pub channel: usize,
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PtzLocate {
    pub enabled: bool,
    pub center_x: i64,
    pub center_y: i64,
    pub width_steps: i64,
    pub height_steps: i64,
    pub motor_speed_x: i64,
    pub motor_speed_y: i64,
}

fn flag(abilities: &Value, path: &[&str]) -> Option<bool> {
    let mut cursor = abilities;
    for segment in path {
        cursor = cursor.get(*segment)?;
    }
    cursor.as_bool()
}

fn flag_capability(abilities: &Value, path: &[&str], blocked: &str) -> Capability {
    match flag(abilities, path) {
        Some(true) => Capability::new(true, format!("device reports {}={}", path.join("."), true)),
        Some(false) => Capability::new(
            false,
            format!("device reports {}={} ({blocked})", path.join("."), false),
        ),
        None => Capability::new(false, format!("device did not report {}", path.join("."))),
    }
}

fn first_record_mode(reply: &Value) -> Option<String> {
    reply
        .get("Record")
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("RecordMode"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn hex_flag(enabled: bool) -> String {
    if enabled {
        "0x00000001".to_string()
    } else {
        "0x00000000".to_string()
    }
}

fn integer(entry: &Value, key: &str) -> i64 {
    entry.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn text(entry: &Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn per_channel<'a>(block: &'a Value, key: &str) -> Vec<(usize, &'a Value)> {
    let mut rows = Vec::new();
    let Some(channels) = block.get(key).and_then(Value::as_array) else {
        return rows;
    };
    for (channel, entries) in channels.iter().enumerate() {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            rows.push((channel, entry));
        }
    }
    rows
}

fn tour_lines_of(uart: &Value) -> Vec<TourLine> {
    per_channel(uart, "PTZTour")
        .into_iter()
        .map(|(channel, entry)| TourLine {
            channel,
            id: integer(entry, "Id"),
            name: text(entry, "Name"),
            points: entry
                .get("Tour")
                .and_then(Value::as_array)
                .map(|points| {
                    points
                        .iter()
                        .map(|point| TourPoint {
                            preset: integer(point, "Id"),
                            seconds: integer(point, "Time"),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

fn presets_of(uart: &Value) -> Vec<PresetEntry> {
    per_channel(uart, "PTZPreset")
        .into_iter()
        .map(|(channel, entry)| PresetEntry {
            channel,
            id: integer(entry, "Id"),
            name: text(entry, "PresetName"),
        })
        .collect()
}

fn locate_of(ptz: &Value) -> Option<PtzLocate> {
    let entry = ptz.get("GunBallLocate")?.as_array()?.first()?;
    Some(PtzLocate {
        enabled: entry
            .get("Enable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        center_x: integer(entry, "PtzLocate_CenterX"),
        center_y: integer(entry, "PtzLocate_CenterY"),
        width_steps: integer(entry, "WidthSteps"),
        height_steps: integer(entry, "HeightSteps"),
        motor_speed_x: integer(entry, "motorSpeedX"),
        motor_speed_y: integer(entry, "motorSpeedY"),
    })
}

fn tour_capability(reported: &Capability, lines: &[TourLine]) -> Capability {
    let configured = lines.iter().map(|line| line.points.len()).sum::<usize>();
    if !reported.supported {
        return reported.clone();
    }
    if configured == 0 {
        return Capability::new(
            false,
            format!(
                "{}; Uart.PTZTour holds no tour line, so a tour would have nothing to run",
                reported.evidence
            ),
        );
    }
    Capability::new(
        true,
        format!(
            "{}; Uart.PTZTour holds {} line(s) with {} point(s) in total",
            reported.evidence,
            lines.len(),
            configured
        ),
    )
}

fn locate_capability(reported: Option<PtzLocate>) -> Capability {
    match reported {
        Some(locate) if locate.enabled => Capability::new(
            true,
            format!(
                "Ptz.GunBallLocate[0].Enable=true, center ({}, {}) with {}x{} steps at motor speed {}x/{}y",
                locate.center_x,
                locate.center_y,
                locate.width_steps,
                locate.height_steps,
                locate.motor_speed_x,
                locate.motor_speed_y
            ),
        ),
        Some(locate) => Capability::new(
            false,
            format!(
                "Ptz.GunBallLocate[0].Enable=false; the device declares the gun and ball pair but has it turned off (center ({}, {}))",
                locate.center_x, locate.center_y
            ),
        ),
        None => Capability::new(false, "the device did not report Ptz.GunBallLocate"),
    }
}

impl Connection {
    fn session_field(&self) -> String {
        format!("0x{:08X}", self.session())
    }

    pub async fn config_get(&mut self, name: &str) -> Result<Value, DvrError> {
        let body = json!({ "Name": name, "SessionID": self.session_field() });
        let reply = self.request(msg::CONFIG_GET, body).await?;
        require_ok(&reply)?;
        Ok(reply)
    }

    pub async fn config_set(&mut self, name: &str, value: Value) -> Result<i64, DvrError> {
        let mut body = json!({ "Name": name, "SessionID": self.session_field() });
        body[name] = value;
        let reply = self.request(msg::CONFIG_SET, body).await?;
        require_ok(&reply)
    }

    pub async fn abilities(&mut self) -> Result<Value, DvrError> {
        let reply = self
            .request(
                msg::ABILITY,
                json!({ "Name": "SystemFunction", "SessionID": self.session_field() }),
            )
            .await?;
        require_ok(&reply)?;
        Ok(reply.get("SystemFunction").cloned().unwrap_or(Value::Null))
    }

    pub async fn system_info(&mut self) -> Result<Value, DvrError> {
        let reply = self
            .request(
                msg::SYSTEM_INFO,
                json!({ "Name": "SystemInfo", "SessionID": self.session_field() }),
            )
            .await?;
        require_ok(&reply)?;
        Ok(reply.get("SystemInfo").cloned().unwrap_or(Value::Null))
    }

    pub async fn set_white_light(&mut self, mode: WhiteLightMode) -> Result<i64, DvrError> {
        let camera = self.config_get("Camera").await?;
        let mut light = camera
            .get("Camera")
            .and_then(|camera| camera.get("WhiteLight"))
            .cloned()
            .filter(Value::is_object)
            .ok_or(DvrError::Rejected {
                ret: 607,
                reason: "this device exposes no Camera.WhiteLight block",
            })?;
        light["WorkMode"] = Value::String(mode.wire().to_string());
        self.config_set("Camera", json!({ "WhiteLight": light }))
            .await
    }

    pub async fn white_light_mode(&mut self) -> Result<Option<String>, DvrError> {
        let camera = self.config_get("Camera").await?;
        Ok(camera
            .get("Camera")
            .and_then(|camera| camera.get("WhiteLight"))
            .and_then(|light| light.get("WorkMode"))
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    pub async fn set_picture_flip(
        &mut self,
        channel: usize,
        flip: Option<bool>,
        mirror: Option<bool>,
    ) -> Result<i64, DvrError> {
        let camera = self.config_get("Camera").await?;
        let mut params = camera
            .get("Camera")
            .and_then(|camera| camera.get("Param"))
            .and_then(Value::as_array)
            .cloned()
            .ok_or(DvrError::Rejected {
                ret: 607,
                reason: "this device exposes no Camera.Param block",
            })?;
        let index = channel.min(params.len().saturating_sub(1));
        let entry = params.get_mut(index).ok_or(DvrError::Rejected {
            ret: 208,
            reason: "camera channel index out of range",
        })?;
        if !entry
            .get("PictureFlip")
            .is_some_and(|value| value.is_string())
        {
            return Err(DvrError::Rejected {
                ret: 605,
                reason: "this device exposes no PictureFlip setting for the channel",
            });
        }
        if let Some(flip) = flip {
            entry["PictureFlip"] = Value::String(hex_flag(flip));
        }
        if let Some(mirror) = mirror {
            entry["PictureMirror"] = Value::String(hex_flag(mirror));
        }
        self.config_set("Camera", json!({ "Param": params })).await
    }

    pub async fn picture_flags(
        &mut self,
        channel: usize,
    ) -> Result<(Option<String>, Option<String>), DvrError> {
        let camera = self.config_get("Camera").await?;
        let params = camera
            .get("Camera")
            .and_then(|camera| camera.get("Param"))
            .and_then(Value::as_array)
            .cloned()
            .filter(|params| !params.is_empty())
            .ok_or(DvrError::Rejected {
                ret: 607,
                reason: "the device returned an empty Camera.Param list, so the picture flip state cannot be read",
            })?;
        let entry = params.get(channel.min(params.len().saturating_sub(1)));
        let read = |key: &str| {
            entry
                .and_then(|entry| entry.get(key))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        Ok((read("PictureFlip"), read("PictureMirror")))
    }

    pub async fn set_record_mode(&mut self, mode: RecordMode) -> Result<i64, DvrError> {
        let record = self.config_get("Record").await?;
        let mut entries = record
            .get("Record")
            .and_then(Value::as_array)
            .cloned()
            .ok_or(DvrError::Rejected {
                ret: 607,
                reason: "this device exposes no Record block",
            })?;
        let entry = entries.get_mut(0).ok_or(DvrError::Rejected {
            ret: 607,
            reason: "this device reports an empty Record block",
        })?;
        entry["RecordMode"] = Value::String(mode.wire().to_string());
        self.config_set("Record", Value::Array(entries)).await
    }

    pub async fn record_mode(&mut self) -> Result<Option<String>, DvrError> {
        let record = self.config_get("Record").await?;
        Ok(first_record_mode(&record))
    }

    pub async fn capabilities(&mut self) -> Result<DeviceCapabilities, DvrError> {
        let abilities = self.abilities().await?;
        let camera = self.config_get("Camera").await.ok();
        let record = self.config_get("Record").await.ok();
        let notice = self.system_info().await.ok();
        let uart = self.config_get("Uart").await.ok();
        let ptz = self.config_get("Ptz").await.ok();

        let white_light = camera
            .as_ref()
            .and_then(|camera| camera.get("Camera"))
            .and_then(|camera| camera.get("WhiteLight"))
            .filter(|light| light.is_object())
            .map(|light| {
                Capability::new(
                    true,
                    format!(
                        "Camera.WhiteLight.WorkMode={}",
                        light.get("WorkMode").and_then(Value::as_str).unwrap_or("?")
                    ),
                )
            })
            .unwrap_or_else(|| {
                flag_capability(
                    &abilities,
                    &["OtherFunction", "SupportCameraWhiteLight"],
                    "no white light block in the camera config",
                )
            });

        let params = camera
            .as_ref()
            .and_then(|camera| camera.get("Camera"))
            .and_then(|camera| camera.get("Param"))
            .and_then(Value::as_array);
        let picture_flip = match params {
            None => Capability::new(false, "device returned no Camera.Param block"),
            Some(params) if params.is_empty() => Capability::new(
                false,
                "device returned an empty Camera.Param list, so PictureFlip cannot be read or written",
            ),
            Some(params) => match params
                .first()
                .and_then(|entry| entry.get("PictureFlip"))
                .and_then(Value::as_str)
            {
                Some(value) => {
                    Capability::new(true, format!("Camera.Param[0].PictureFlip={value}"))
                }
                None => Capability::new(
                    false,
                    "Camera.Param[0] carries no PictureFlip field on this firmware",
                ),
            },
        };

        let record_mode = record
            .as_ref()
            .and_then(first_record_mode)
            .map(|mode| Capability::new(true, format!("Record[0].RecordMode={mode}")))
            .unwrap_or_else(|| Capability::new(false, "device reports no Record[0].RecordMode"));

        let uart_block = uart
            .as_ref()
            .and_then(|reply| reply.get("Uart"))
            .unwrap_or(&Value::Null);
        let tour_lines = tour_lines_of(uart_block);
        let locate = ptz
            .as_ref()
            .and_then(|reply| reply.get("Ptz"))
            .and_then(locate_of);

        Ok(DeviceCapabilities {
            device_type: notice
                .as_ref()
                .and_then(|info| info.get("DeviceType"))
                .and_then(Value::as_str)
                .map(str::to_string),
            white_light,
            picture_flip,
            snapshot: flag_capability(
                &abilities,
                &["EncodeFunction", "SnapStream"],
                "no snapshot stream reported",
            ),
            record_mode,
            talk: flag_capability(
                &abilities,
                &["OtherFunction", "SupportTwoWayVoiceTalk"],
                "no two-way audio reported",
            ),
            ptz_tour: tour_capability(
                &flag_capability(
                    &abilities,
                    &["OtherFunction", "SupportPTZTour"],
                    "no patrol support reported",
                ),
                &tour_lines,
            ),
            ptz_locate: locate_capability(locate),
            ptz_direction: flag_capability(
                &abilities,
                &["OtherFunction", "SupportPTZDirectionControl"],
                "no PTZ direction control reported",
            ),
            tour_lines,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtzDirection {
    Up,
    Down,
    Left,
    Right,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
}

impl PtzDirection {
    pub fn from_velocity(pan: f32, tilt: f32) -> Option<Self> {
        let horizontal = if pan > 0.05 {
            Some(1i8)
        } else if pan < -0.05 {
            Some(-1i8)
        } else {
            None
        };
        let vertical = if tilt > 0.05 {
            Some(1i8)
        } else if tilt < -0.05 {
            Some(-1i8)
        } else {
            None
        };
        Some(match (horizontal, vertical) {
            (None, None) => return None,
            (Some(1), None) => PtzDirection::Right,
            (Some(-1), None) => PtzDirection::Left,
            (None, Some(1)) => PtzDirection::Up,
            (None, Some(-1)) => PtzDirection::Down,
            (Some(1), Some(1)) => PtzDirection::UpRight,
            (Some(1), Some(-1)) => PtzDirection::DownRight,
            (Some(-1), Some(1)) => PtzDirection::UpLeft,
            (Some(-1), Some(-1)) => PtzDirection::DownLeft,
            _ => return None,
        })
    }

    pub fn wire(self) -> &'static str {
        match self {
            PtzDirection::Up => "DirectionUp",
            PtzDirection::Down => "DirectionDown",
            PtzDirection::Left => "DirectionLeft",
            PtzDirection::Right => "DirectionRight",
            PtzDirection::UpLeft => "DirectionLeftUp",
            PtzDirection::UpRight => "DirectionRightUp",
            PtzDirection::DownLeft => "DirectionLeftDown",
            PtzDirection::DownRight => "DirectionRightDown",
        }
    }
}

impl Connection {
    async fn ptz_control(
        &mut self,
        command: &str,
        channel: usize,
        mut parameter: Value,
    ) -> Result<i64, DvrError> {
        parameter["Channel"] = json!(channel);
        let body = json!({
            "Name": "OPPTZControl",
            "SessionID": self.session_field(),
            "OPPTZControl": {
                "Command": command,
                "Parameter": parameter,
            },
        });
        let reply = self.request(msg::PTZ_CONTROL, body).await?;
        require_ok(&reply)
    }

    async fn ptz_command(
        &mut self,
        direction: PtzDirection,
        step: u8,
        preset: i32,
    ) -> Result<i64, DvrError> {
        let parameter = json!({
            "AUX": { "Number": 0, "Status": "On" },
            "MenuOpts": "Enter",
            "Pattern": "SetBegin",
            "Preset": preset,
            "Step": step,
            "Tour": 0,
        });
        self.ptz_control(direction.wire(), 0, parameter).await
    }

    pub async fn tour_listing(&mut self) -> Result<(Vec<TourLine>, Vec<PresetEntry>), DvrError> {
        let reply = self.config_get("Uart").await?;
        let block = reply.get("Uart").unwrap_or(&Value::Null);
        Ok((tour_lines_of(block), presets_of(block)))
    }

    pub async fn ptz_locate(&mut self) -> Result<Option<PtzLocate>, DvrError> {
        let reply = self.config_get("Ptz").await?;
        Ok(reply.get("Ptz").and_then(locate_of))
    }

    pub async fn add_tour_point(
        &mut self,
        channel: usize,
        tour: i64,
        preset: i64,
        index: i64,
    ) -> Result<(), DvrError> {
        let parameter = json!({
            "Preset": preset,
            "PresetIndex": index,
            "Step": 3,
            "Tour": tour,
            "TourTimes": 1,
        });
        self.ptz_control("AddTour", channel, parameter)
            .await
            .map(|_| ())
    }

    pub async fn clear_tour(&mut self, channel: usize, tour: i64) -> Result<(), DvrError> {
        let parameter = json!({ "Step": 3, "Tour": tour });
        self.ptz_control("ClearTour", channel, parameter)
            .await
            .map(|_| ())
    }

    pub async fn start_tour(&mut self, channel: usize, tour: i64) -> Result<(), DvrError> {
        let parameter = json!({ "Step": 3, "Tour": tour, "TourTimes": 1 });
        self.ptz_control("StartTour", channel, parameter)
            .await
            .map(|_| ())
    }

    pub async fn stop_tour(&mut self, channel: usize, tour: i64) -> Result<(), DvrError> {
        let parameter = json!({ "Step": 3, "Tour": tour });
        self.ptz_control("StopTour", channel, parameter)
            .await
            .map(|_| ())
    }

    pub async fn ptz_step(
        &mut self,
        pan: f32,
        tilt: f32,
        hold: std::time::Duration,
    ) -> Result<(), DvrError> {
        let direction = PtzDirection::from_velocity(pan, tilt).ok_or(DvrError::Rejected {
            ret: 208,
            reason: "a ptz move needs a pan or tilt velocity",
        })?;
        let magnitude = pan.abs().max(tilt.abs()).clamp(0.0, 1.0);
        let step = (magnitude * 8.0).round().max(1.0) as u8;
        self.ptz_command(direction, step, 65535).await?;
        tokio::time::sleep(hold).await;
        self.ptz_command(direction, step, -1).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvrip::fake::{connect, json_reply, serve};

    #[test]
    fn modes_use_the_wire_strings_the_device_reports() {
        assert_eq!(WhiteLightMode::Always.wire(), "Always");
        assert_eq!(
            WhiteLightMode::parse("intelligent"),
            Some(WhiteLightMode::Intelligent)
        );
        assert_eq!(RecordMode::Manual.wire(), "ManualRecord");
        assert_eq!(RecordMode::parse("schedule"), Some(RecordMode::Config));
        assert_eq!(RecordMode::parse("nonsense"), None);
    }

    #[test]
    fn capability_flags_quote_the_device_wording() {
        let abilities = json!({
            "OtherFunction": {
                "SupportCameraWhiteLight": true,
                "SupportPTZTour": false,
            }
        });
        let light = flag_capability(
            &abilities,
            &["OtherFunction", "SupportCameraWhiteLight"],
            "x",
        );
        assert!(light.supported);
        assert!(light
            .evidence
            .contains("OtherFunction.SupportCameraWhiteLight=true"));
        let tour = flag_capability(
            &abilities,
            &["OtherFunction", "SupportPTZTour"],
            "no patrol",
        );
        assert!(!tour.supported);
        assert!(tour.evidence.contains("no patrol"));
        let missing = flag_capability(&abilities, &["OtherFunction", "SupportNope"], "x");
        assert!(!missing.supported);
        assert!(missing.evidence.contains("did not report"));
    }

    #[test]
    fn hex_flags_round_trip_through_the_config_strings() {
        assert_eq!(hex_flag(true), "0x00000001");
        assert_eq!(hex_flag(false), "0x00000000");
    }

    fn uart_with_tour() -> Value {
        json!({
            "PTZTour": [
                [
                    {
                        "Id": 1,
                        "Name": "",
                        "Tour": [
                            { "Id": 1, "Name": "", "Time": 3 },
                            { "Id": 2, "Name": "", "Time": 3 },
                        ],
                    }
                ]
            ],
            "PTZPreset": [[
                { "Id": 1, "PresetName": "biasa" },
                { "Id": 2, "PresetName": "biasabaru" },
            ]],
        })
    }

    #[test]
    fn a_null_tour_block_reads_as_no_lines() {
        let uart = json!({ "PTZTour": null, "PTZPreset": null });
        assert!(tour_lines_of(&uart).is_empty());
        assert!(presets_of(&uart).is_empty());
    }

    #[test]
    fn the_tour_block_carries_the_presets_it_cycles() {
        let lines = tour_lines_of(&uart_with_tour());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].id, 1);
        assert_eq!(lines[0].channel, 0);
        assert_eq!(
            lines[0].points,
            vec![
                TourPoint {
                    preset: 1,
                    seconds: 3
                },
                TourPoint {
                    preset: 2,
                    seconds: 3
                },
            ]
        );
        let presets = presets_of(&uart_with_tour());
        assert_eq!(presets.len(), 2);
        assert_eq!(presets[1].name, "biasabaru");
        assert_eq!(presets[1].id, 2);
    }

    #[test]
    fn the_tour_capability_needs_a_line_that_can_move() {
        let reported = Capability::new(true, "device reports OtherFunction.SupportPTZTour=true");
        let empty = tour_capability(&reported, &[]);
        assert!(!empty.supported);
        assert!(empty.evidence.contains("no tour line"));

        let lines = tour_lines_of(&uart_with_tour());
        let ready = tour_capability(&reported, &lines);
        assert!(ready.supported);
        assert!(ready.evidence.contains("1 line(s) with 2 point(s)"));

        let unsupported =
            Capability::new(false, "device reports OtherFunction.SupportPTZTour=false");
        let refused = tour_capability(&unsupported, &lines);
        assert!(!refused.supported);
        assert_eq!(refused.evidence, unsupported.evidence);
    }

    #[test]
    fn the_locate_block_names_the_gun_ball_pair() {
        let ptz = json!({
            "GunBallLocate": [{
                "AdjustHeightSteps": 305,
                "AdjustWidthSteps": 1010,
                "Enable": true,
                "HeightSteps": 285,
                "PtzLocate_CenterX": 1735,
                "PtzLocate_CenterY": 1028,
                "WidthSteps": 515,
                "motorSpeedX": 13,
                "motorSpeedY": 9,
            }]
        });
        let locate = locate_of(&ptz).expect("the captured block parses");
        assert!(locate.enabled);
        assert_eq!(locate.center_x, 1735);
        assert_eq!(locate.motor_speed_y, 9);
        let capability = locate_capability(Some(locate));
        assert!(capability.supported);
        assert!(capability.evidence.contains("center (1735, 1028)"));
        assert!(capability.evidence.contains("515x285"));

        assert!(locate_of(&json!({ "GunBallLocate": null })).is_none());
        assert!(locate_of(&json!({})).is_none());
        assert!(!locate_capability(None).supported);
    }

    #[tokio::test]
    async fn the_tour_wire_carries_the_preset_index_and_one_tour_time() {
        let (address, handle) = serve(vec![
            json_reply(json!({ "Ret": 100 })),
            json_reply(json!({ "Ret": 100 })),
        ])
        .await;
        let mut connection = connect(&address).await;
        connection
            .add_tour_point(0, 3, 2, 1)
            .await
            .expect("AddTour is acked");
        connection
            .start_tour(0, 3)
            .await
            .expect("StartTour is acked");
        let sent = handle.await.expect("the fake device answers both frames");
        assert_eq!(sent.len(), 2);

        let add: Value = serde_json::from_str(sent[0].trim_end_matches(['\0', '\n']).trim())
            .expect("AddTour json");
        assert_eq!(add["OPPTZControl"]["Command"], "AddTour");
        let parameter = &add["OPPTZControl"]["Parameter"];
        assert_eq!(parameter["Preset"], 2);
        assert_eq!(parameter["PresetIndex"], 1);
        assert_eq!(parameter["Tour"], 3);
        assert_eq!(parameter["Step"], 3);
        assert_eq!(parameter["TourTimes"], 1);
        assert_eq!(parameter["Channel"], 0);

        let start: Value = serde_json::from_str(sent[1].trim_end_matches(['\0', '\n']).trim())
            .expect("StartTour json");
        assert_eq!(start["OPPTZControl"]["Command"], "StartTour");
        let parameter = &start["OPPTZControl"]["Parameter"];
        assert_eq!(parameter["Tour"], 3);
        assert_eq!(parameter["TourTimes"], 1);
        assert!(parameter.get("Preset").is_none());
    }

    #[test]
    fn a_disabled_locate_block_is_not_a_motorised_lens() {
        let locate = PtzLocate {
            enabled: false,
            center_x: 1,
            center_y: 2,
            width_steps: 3,
            height_steps: 4,
            motor_speed_x: 5,
            motor_speed_y: 6,
        };
        let capability = locate_capability(Some(locate));
        assert!(!capability.supported);
        assert!(capability.evidence.contains("Enable=false"));
    }
}
