use serde::Serialize;
use serde_json::{json, Value};

use super::client::{require_ok, Connection, DvrError};
use super::control::{PresetEntry, PtzDirection};
use super::msg;

pub const LOCATE_SPAN: i32 = 4096;
pub const PRESET_NAME_MAX: usize = 16;
pub const PRESET_LIMIT: usize = 32;
pub const PRESET_WORD: &str = "Preset";
pub const PRESS_STEP: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtzAction {
    Direction(PtzDirection),
    ZoomIn,
    ZoomOut,
    FocusNear,
    FocusFar,
}

impl PtzAction {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "up" => PtzAction::Direction(PtzDirection::Up),
            "down" => PtzAction::Direction(PtzDirection::Down),
            "left" => PtzAction::Direction(PtzDirection::Left),
            "right" => PtzAction::Direction(PtzDirection::Right),
            "up-left" => PtzAction::Direction(PtzDirection::UpLeft),
            "up-right" => PtzAction::Direction(PtzDirection::UpRight),
            "down-left" => PtzAction::Direction(PtzDirection::DownLeft),
            "down-right" => PtzAction::Direction(PtzDirection::DownRight),
            "zoom-in" => PtzAction::ZoomIn,
            "zoom-out" => PtzAction::ZoomOut,
            "focus-near" => PtzAction::FocusNear,
            "focus-far" => PtzAction::FocusFar,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            PtzAction::Direction(PtzDirection::Up) => "up",
            PtzAction::Direction(PtzDirection::Down) => "down",
            PtzAction::Direction(PtzDirection::Left) => "left",
            PtzAction::Direction(PtzDirection::Right) => "right",
            PtzAction::Direction(PtzDirection::UpLeft) => "up-left",
            PtzAction::Direction(PtzDirection::UpRight) => "up-right",
            PtzAction::Direction(PtzDirection::DownLeft) => "down-left",
            PtzAction::Direction(PtzDirection::DownRight) => "down-right",
            PtzAction::ZoomIn => "zoom-in",
            PtzAction::ZoomOut => "zoom-out",
            PtzAction::FocusNear => "focus-near",
            PtzAction::FocusFar => "focus-far",
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            PtzAction::Direction(direction) => direction.wire(),
            PtzAction::ZoomIn => "ZoomTile",
            PtzAction::ZoomOut => "ZoomWide",
            PtzAction::FocusNear => "FocusNear",
            PtzAction::FocusFar => "FocusFar",
        }
    }

    pub fn moves_the_lens(self) -> bool {
        !matches!(self, PtzAction::Direction(_))
    }
}

pub const ZOOM_ABILITY: [&str; 2] = ["OtherFunction", "SupportBallTelescopic"];
pub const LOCATE_ABILITY: [&str; 2] = ["OtherFunction", "SupportGunBallTwoSensorPtzLocate"];
pub const PRESET_ABILITY: [&str; 2] = ["OtherFunction", "SupportPTZTour"];
pub const CALIBRATE_ABILITY: [&str; 2] = ["OtherFunction", "SupportPtzAutoAdjust"];

pub fn ability_refusal(abilities: &Value, path: &[&str]) -> Option<String> {
    let name = path.join(".");
    let mut cursor = abilities;
    for segment in path {
        match cursor.get(*segment) {
            Some(next) => cursor = next,
            None => return Some(format!("device did not report {name}")),
        }
    }
    match cursor.as_bool() {
        Some(true) => None,
        Some(false) => Some(format!("device reports {name}=false")),
        None => Some(format!("device reports {name}={cursor}, not true")),
    }
}

pub fn locate_offsets(x: f64, y: f64) -> Option<(i32, i32)> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let span = f64::from(LOCATE_SPAN);
    let xoffset = (x * 2.0 * span - span) as i32;
    let yoffset = -((y * 2.0 * span - span) as i32);
    let inside = |value: i32| (-LOCATE_SPAN..=LOCATE_SPAN).contains(&value);
    (inside(xoffset) && inside(yoffset)).then_some((xoffset, yoffset))
}

pub fn preset_name(raw: Option<&str>, id: i64) -> String {
    let trimmed = raw.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return format!("{PRESET_WORD}{id}");
    }
    let mut units = 0;
    trimmed
        .chars()
        .take_while(|letter| {
            units += letter.len_utf16();
            units <= PRESET_NAME_MAX
        })
        .collect()
}

pub fn next_preset_id(presets: &[PresetEntry]) -> i64 {
    let mut id = 1;
    while presets.iter().any(|preset| preset.id == id) {
        id += 1;
    }
    id
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PanoramaRange {
    pub hor_start: i64,
    pub hor_steps: i64,
    pub hor_now: i64,
    pub ver_start: i64,
    pub ver_steps: i64,
    pub ver_now: i64,
}

fn number(block: &Value, key: &str) -> i64 {
    block.get(key).and_then(Value::as_i64).unwrap_or(0)
}

pub fn panorama_of(reply: &Value) -> Option<PanoramaRange> {
    let raw = reply.get("GoToPosition")?;
    let block = match raw {
        Value::Array(entries) => entries.first()?,
        other => other,
    };
    let range = PanoramaRange {
        hor_start: number(block, "HorStart"),
        hor_steps: number(block, "HorSteps"),
        hor_now: number(block, "HorNow"),
        ver_start: number(block, "VerStart"),
        ver_steps: number(block, "VerSteps"),
        ver_now: number(block, "VerNow"),
    };
    (range.hor_steps > 0).then_some(range)
}

impl Connection {
    async fn ptz_action(
        &mut self,
        action: PtzAction,
        step: u8,
        preset: i32,
    ) -> Result<(), DvrError> {
        let parameter = json!({
            "AUX": { "Number": 0, "Status": "On" },
            "MenuOpts": "Enter",
            "Pattern": "SetBegin",
            "Preset": preset,
            "Step": step,
            "Tour": 0,
        });
        self.ptz_control(action.wire(), 0, parameter)
            .await
            .map(|_| ())
    }

    pub async fn ptz_press(&mut self, action: PtzAction, step: u8) -> Result<(), DvrError> {
        self.ptz_action(action, step, 65535).await
    }

    pub async fn ptz_release(&mut self, action: PtzAction, step: u8) -> Result<(), DvrError> {
        self.ptz_action(action, step, -1).await
    }

    pub async fn ptz_locate_to(&mut self, xoffset: i32, yoffset: i32) -> Result<(), DvrError> {
        let body = json!({
            "Name": "OPPtzLocate",
            "SessionID": self.session_field(),
            "OPPtzLocate": [{ "xoffset": xoffset, "yoffset": yoffset }],
        });
        let reply = self.request(msg::PTZ_LOCATE, body).await?;
        require_ok(&reply).map(|_| ())
    }

    pub async fn set_preset(&mut self, id: i64, name: &str) -> Result<(), DvrError> {
        let parameter = json!({ "Preset": id, "PresetName": name });
        self.ptz_control("SetPreset", 0, parameter)
            .await
            .map(|_| ())
    }

    pub async fn rename_preset(&mut self, id: i64, name: &str) -> Result<(), DvrError> {
        let parameter = json!({ "Preset": id, "PresetName": name });
        self.ptz_control("SetPresetName", 0, parameter)
            .await
            .map(|_| ())
    }

    pub async fn clear_preset(&mut self, id: i64) -> Result<(), DvrError> {
        self.ptz_control("ClearPreset", 0, json!({ "Preset": id }))
            .await
            .map(|_| ())
    }

    pub async fn delete_tour_point(
        &mut self,
        channel: usize,
        tour: i64,
        preset: i64,
    ) -> Result<(), DvrError> {
        let parameter = json!({ "Preset": preset, "Step": 3, "Tour": tour, "TourTimes": 1 });
        self.ptz_control("DeleteTour", channel, parameter)
            .await
            .map(|_| ())
    }

    pub async fn ptz_auto_adjust(&mut self) -> Result<(), DvrError> {
        let body = json!({ "Name": "OPPtzAutoAdjust", "SessionID": self.session_field() });
        let reply = self.request(msg::OP_MACHINE, body).await?;
        require_ok(&reply).map(|_| ())
    }

    pub async fn panorama_range(&mut self) -> Result<Option<PanoramaRange>, DvrError> {
        let body = json!({ "Name": "GoToPosition", "SessionID": self.session_field() });
        let reply = self.request(msg::ABILITY, body).await?;
        require_ok(&reply)?;
        Ok(panorama_of(&reply))
    }

    pub async fn goto_position(&mut self, left: i64, top: i64) -> Result<(), DvrError> {
        let body = json!({
            "Name": "OPPTZControl",
            "SessionID": self.session_field(),
            "OPPTZControl": {
                "Command": "GoToPosition",
                "Parameter": { "POINT": { "left": left, "top": top } },
            },
        });
        let reply = self.request(msg::PTZ_CONTROL, body).await?;
        require_ok(&reply).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvrip::fake::{connect, json_reply, serve};

    fn body(frame: &str) -> Value {
        serde_json::from_str(frame.trim_end_matches(['\0', '\n', ' '])).expect("json frame")
    }

    #[test]
    fn an_ability_counts_only_when_the_device_reports_true() {
        let abilities = json!({ "OtherFunction": { "SupportPTZTour": true, "SupportBallTelescopic": false, "SupportPtzAutoAdjust": 1 } });
        assert_eq!(ability_refusal(&abilities, &PRESET_ABILITY), None);
        assert_eq!(
            ability_refusal(&abilities, &ZOOM_ABILITY).as_deref(),
            Some("device reports OtherFunction.SupportBallTelescopic=false")
        );
        assert_eq!(
            ability_refusal(&abilities, &LOCATE_ABILITY).as_deref(),
            Some("device did not report OtherFunction.SupportGunBallTwoSensorPtzLocate")
        );
        assert!(ability_refusal(&abilities, &CALIBRATE_ABILITY).is_some());
        assert!(ability_refusal(&Value::Null, &PRESET_ABILITY).is_some());
    }

    #[test]
    fn a_click_maps_to_the_offsets_monitor_activity_sends() {
        assert_eq!(locate_offsets(0.5, 0.5), Some((0, 0)));
        assert_eq!(locate_offsets(0.0, 0.0), Some((-4096, 4096)));
        assert_eq!(locate_offsets(1.0, 1.0), Some((4096, -4096)));
        assert_eq!(locate_offsets(0.25, 0.75), Some((-2048, -2048)));
        assert_eq!(locate_offsets(0.75, 0.125), Some((2048, 3072)));
        assert_eq!(locate_offsets(0.50001, 0.49999), Some((0, 0)));
        assert_eq!(locate_offsets(1.2, 0.5), None);
        assert_eq!(locate_offsets(0.5, -0.1), None);
        assert_eq!(locate_offsets(f64::NAN, 0.5), None);
    }

    #[test]
    fn preset_names_follow_the_preset_edit_rules() {
        assert_eq!(preset_name(None, 3), "Preset3");
        assert_eq!(preset_name(Some("   "), 7), "Preset7");
        assert_eq!(preset_name(Some(" Gate "), 3), "Gate");
        assert_eq!(
            preset_name(Some("abcdefghijklmnopqrst"), 3),
            "abcdefghijklmnop"
        );
        assert_eq!(
            preset_name(Some("ééééééééééééééééé"), 3).chars().count(),
            16
        );
        assert_eq!(
            preset_name(Some("a😀😀😀😀😀😀😀😀😀"), 3),
            "a😀😀😀😀😀😀😀"
        );
    }

    #[test]
    fn a_new_preset_takes_the_lowest_free_id() {
        let entry = |id| PresetEntry {
            channel: 0,
            id,
            name: String::new(),
        };
        assert_eq!(next_preset_id(&[]), 1);
        assert_eq!(next_preset_id(&[entry(1), entry(2), entry(250)]), 3);
        assert_eq!(next_preset_id(&[entry(2), entry(3)]), 1);
        assert_eq!(next_preset_id(&[entry(1), entry(3)]), 2);
    }

    #[test]
    fn actions_carry_the_wire_names_the_sdk_sends() {
        assert_eq!(
            PtzAction::parse("zoom-in").map(PtzAction::wire),
            Some("ZoomTile")
        );
        assert_eq!(
            PtzAction::parse("zoom-out").map(PtzAction::wire),
            Some("ZoomWide")
        );
        assert_eq!(
            PtzAction::parse("focus-near").map(PtzAction::wire),
            Some("FocusNear")
        );
        assert_eq!(
            PtzAction::parse("focus-far").map(PtzAction::wire),
            Some("FocusFar")
        );
        assert_eq!(
            PtzAction::parse("LEFT").map(PtzAction::wire),
            Some("DirectionLeft")
        );
        assert_eq!(
            PtzAction::parse("up-right").map(PtzAction::wire),
            Some("DirectionRightUp")
        );
        assert_eq!(PtzAction::parse("spin"), None);
        for name in ["up", "down-left", "zoom-out", "focus-far"] {
            assert_eq!(PtzAction::parse(name).map(PtzAction::name), Some(name));
        }
        assert!(PtzAction::ZoomIn.moves_the_lens());
        assert!(!PtzAction::Direction(PtzDirection::Up).moves_the_lens());
    }

    #[test]
    fn the_panorama_range_reads_object_or_array_replies() {
        let block = json!({ "HorStart": 0, "HorSteps": 3600, "HorNow": 900, "VerStart": 40, "VerSteps": 90, "VerNow": 45 });
        let object = panorama_of(&json!({ "Ret": 100, "GoToPosition": block.clone() })).unwrap();
        assert_eq!(object.hor_steps, 3600);
        assert_eq!(object.ver_start, 40);
        assert_eq!(
            panorama_of(&json!({ "GoToPosition": [block] })),
            Some(object)
        );
        assert_eq!(
            panorama_of(&json!({ "GoToPosition": { "HorSteps": 0 } })),
            None
        );
        assert_eq!(panorama_of(&json!({ "Ret": 100 })), None);
    }

    #[tokio::test]
    async fn press_and_release_send_the_same_command_with_start_and_stop_presets() {
        let (peer, server) = serve(vec![
            json_reply(json!({ "Ret": 100 })),
            json_reply(json!({ "Ret": 100 })),
        ])
        .await;
        let mut connection = connect(&peer).await;
        connection
            .ptz_press(PtzAction::ZoomIn, PRESS_STEP)
            .await
            .unwrap();
        connection
            .ptz_release(PtzAction::ZoomIn, PRESS_STEP)
            .await
            .unwrap();
        let seen = server.await.unwrap();
        let start = body(&seen[0]);
        let stop = body(&seen[1]);
        assert_eq!(start["OPPTZControl"]["Command"], "ZoomTile");
        assert_eq!(stop["OPPTZControl"]["Command"], "ZoomTile");
        assert_eq!(start["OPPTZControl"]["Parameter"]["Preset"], 65535);
        assert_eq!(stop["OPPTZControl"]["Parameter"]["Preset"], -1);
        assert_eq!(start["OPPTZControl"]["Parameter"]["Step"], 4);
        assert_eq!(stop["OPPTZControl"]["Parameter"]["Step"], 4);
        assert_eq!(start["OPPTZControl"]["Parameter"]["Channel"], 0);
    }

    #[tokio::test]
    async fn preset_authoring_uses_the_preset_page_commands() {
        let (peer, server) = serve(vec![
            json_reply(json!({ "Ret": 100 })),
            json_reply(json!({ "Ret": 100 })),
            json_reply(json!({ "Ret": 100 })),
            json_reply(json!({ "Ret": 100 })),
        ])
        .await;
        let mut connection = connect(&peer).await;
        connection.set_preset(3, "Preset3").await.unwrap();
        connection.rename_preset(3, "Porch").await.unwrap();
        connection.delete_tour_point(0, 1, 3).await.unwrap();
        connection.clear_preset(3).await.unwrap();
        let seen: Vec<Value> = server
            .await
            .unwrap()
            .iter()
            .map(|frame| body(frame))
            .collect();
        let commands: Vec<&str> = seen
            .iter()
            .map(|frame| frame["OPPTZControl"]["Command"].as_str().unwrap())
            .collect();
        assert_eq!(
            commands,
            ["SetPreset", "SetPresetName", "DeleteTour", "ClearPreset"]
        );
        assert_eq!(
            seen[0]["OPPTZControl"]["Parameter"]["PresetName"],
            "Preset3"
        );
        assert_eq!(seen[1]["OPPTZControl"]["Parameter"]["PresetName"], "Porch");
        assert_eq!(seen[1]["OPPTZControl"]["Parameter"]["Preset"], 3);
        assert_eq!(seen[2]["OPPTZControl"]["Parameter"]["Tour"], 1);
        assert_eq!(seen[2]["OPPTZControl"]["Parameter"]["Preset"], 3);
        assert_eq!(seen[2]["OPPTZControl"]["Parameter"]["Step"], 3);
        assert_eq!(seen[3]["OPPTZControl"]["Parameter"]["Preset"], 3);
        assert_eq!(seen[3]["OPPTZControl"]["Parameter"]["Channel"], 0);
    }

    #[tokio::test]
    async fn locate_sends_one_offset_pair_under_its_own_name() {
        let (peer, server) = serve(vec![json_reply(json!({ "Ret": 100 }))]).await;
        let mut connection = connect(&peer).await;
        connection.ptz_locate_to(-2048, 1024).await.unwrap();
        let seen = server.await.unwrap();
        let frame = body(&seen[0]);
        assert_eq!(frame["Name"], "OPPtzLocate");
        assert_eq!(
            frame["OPPtzLocate"],
            json!([{ "xoffset": -2048, "yoffset": 1024 }])
        );
    }

    #[tokio::test]
    async fn a_refused_command_surfaces_the_device_code() {
        let (peer, _server) = serve(vec![json_reply(json!({ "Ret": 607 }))]).await;
        let mut connection = connect(&peer).await;
        let error = connection.ptz_auto_adjust().await.unwrap_err();
        assert!(
            matches!(error, DvrError::Rejected { ret: 607, .. }),
            "{error:?}"
        );
    }
}
