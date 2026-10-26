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
            ptz_tour: flag_capability(
                &abilities,
                &["OtherFunction", "SupportPTZTour"],
                "no patrol support reported",
            ),
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
    async fn ptz_command(
        &mut self,
        direction: PtzDirection,
        step: u8,
        preset: i32,
    ) -> Result<i64, DvrError> {
        let body = json!({
            "Name": "OPPTZControl",
            "SessionID": self.session_field(),
            "OPPTZControl": {
                "Command": direction.wire(),
                "Parameter": {
                    "AUX": { "Number": 0, "Status": "On" },
                    "Channel": 0,
                    "MenuOpts": "Enter",
                    "Pattern": "SetBegin",
                    "Preset": preset,
                    "Step": step,
                    "Tour": 0,
                },
            },
        });
        let reply = self.request(msg::PTZ_CONTROL, body).await?;
        require_ok(&reply)
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
}
