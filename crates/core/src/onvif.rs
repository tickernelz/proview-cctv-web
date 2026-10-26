use std::time::Duration;

use serde::Serialize;

const DEVICE_NS: &str = "http://www.onvif.org/ver10/device/wsdl";
const MEDIA_NS: &str = "http://www.onvif.org/ver10/media/wsdl";
const PTZ_NS: &str = "http://www.onvif.org/ver20/ptz/wsdl";
const IMAGING_NS: &str = "http://www.onvif.org/ver20/imaging/wsdl";
const SCHEMA_NS: &str = "http://www.onvif.org/ver10/schema";

#[derive(Debug)]
pub enum OnvifError {
    Transport(String),
    Fault {
        status: u16,
        code: String,
        reason: String,
    },
    Missing {
        what: &'static str,
    },
    Blocked {
        operation: &'static str,
        reason: &'static str,
    },
}

impl std::fmt::Display for OnvifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnvifError::Transport(detail) => write!(f, "onvif transport: {detail}"),
            OnvifError::Fault {
                status,
                code,
                reason,
            } => {
                write!(f, "onvif fault HTTP {status}: {code} ({reason})")
            }
            OnvifError::Missing { what } => write!(f, "onvif reply has no {what}"),
            OnvifError::Blocked { operation, reason } => {
                write!(f, "{operation} is blocked in code: {reason}")
            }
        }
    }
}

impl std::error::Error for OnvifError {}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceInformation {
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub serial: String,
    pub hardware: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Profile {
    pub token: String,
    pub name: String,
    pub video_source: Option<String>,
    pub encoder: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PtzNode {
    pub token: String,
    pub max_presets: u32,
    pub home_supported: bool,
    pub auxiliary_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Preset {
    pub token: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EncoderConfiguration {
    pub token: String,
    pub name: String,
    pub encoding: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
    pub bitrate_kbps: u32,
    pub gop: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NetworkInterface {
    pub token: String,
    pub name: String,
    pub enabled: bool,
    pub ipv4: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MoveTiming {
    pub requested: Duration,
    pub clamped_to_device_limit: bool,
}

fn local_name(raw: &str) -> &str {
    match raw.rfind(':') {
        Some(index) => &raw[index + 1..],
        None => raw,
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=");
    let start = tag.find(&marker)? + marker.len();
    let quote = tag[start..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value_start = start + quote.len_utf8();
    let value_end = value_start + tag[value_start..].find(quote)?;
    Some(tag[value_start..value_end].to_string())
}

fn text_of(xml: &str, element: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(offset) = xml[cursor..].find('<') {
        let start = cursor + offset;
        let rest = &xml[start + 1..];
        if rest.starts_with('/') {
            cursor = start + 1;
            continue;
        }
        let end = rest.find('>')?;
        let tag = &rest[..end];
        let name_end = tag
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(tag.len());
        if local_name(&tag[..name_end]) == element {
            let value_start = start + 1 + end + 1;
            let closing = xml[value_start..].find('<')?;
            return Some(xml[value_start..value_start + closing].trim().to_string());
        }
        cursor = start + 1;
    }
    None
}

fn number_of(xml: &str, element: &str) -> Option<u32> {
    text_of(xml, element).and_then(|text| text.parse().ok())
}

fn block_of<'a>(xml: &'a str, element: &str) -> Option<&'a str> {
    let mut cursor = 0;
    while let Some(offset) = xml[cursor..].find('<') {
        let start = cursor + offset;
        let rest = &xml[start + 1..];
        if rest.starts_with('/') {
            cursor = start + 1;
            continue;
        }
        let end = rest.find('>')?;
        let tag = &rest[..end];
        let name_end = tag
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(tag.len());
        if local_name(&tag[..name_end]) == element {
            let body_start = start + 1 + end + 1;
            let closing = format!("</{}", &tag[..name_end]);
            let body_end = xml[body_start..].find(&closing)?;
            return Some(&xml[body_start..body_start + body_end]);
        }
        cursor = start + 1;
    }
    None
}

fn segments<'a>(xml: &'a str, marker: &str) -> Vec<&'a str> {
    let mut parts: Vec<&str> = xml.split(marker).collect();
    if parts.is_empty() {
        return Vec::new();
    }
    parts.remove(0);
    parts
}

fn element_token(xml: &str, element: &str) -> Option<String> {
    let marker = format!("{element} token=");
    quoted(segments(xml, &marker).first()?)
}

fn quoted(raw: &str) -> Option<String> {
    let quote = raw.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let start = quote.len_utf8();
    let end = start + raw[start..].find(quote)?;
    Some(raw[start..end].to_string())
}

fn fault(xml: &str, status: u16) -> OnvifError {
    let code = text_of(xml, "Value")
        .or_else(|| text_of(xml, "Subcode"))
        .unwrap_or_else(|| "unknown".to_string());
    let reason = text_of(xml, "Text").unwrap_or_else(|| "no reason given".to_string());
    OnvifError::Fault {
        status,
        code,
        reason,
    }
}

pub struct Client {
    endpoint: String,
    profile: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(endpoint: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/0.1 onvif")
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(0)
            .build()
            .expect("http client");
        Self {
            endpoint: endpoint.into(),
            profile: "000".to_string(),
            http,
        }
    }

    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = profile.into();
        self
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    fn envelope(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>{body}</s:Body></s:Envelope>"#
        )
    }

    async fn call(&self, body: String) -> Result<String, OnvifError> {
        let response = self
            .http
            .post(&self.endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/soap+xml; charset=utf-8",
            )
            .body(Self::envelope(&body))
            .send()
            .await
            .map_err(|error| OnvifError::Transport(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| OnvifError::Transport(error.to_string()))?;
        if status.is_success() {
            return Ok(text);
        }
        Err(fault(&text, status.as_u16()))
    }

    pub async fn device_information(&self) -> Result<DeviceInformation, OnvifError> {
        let reply = self
            .call(format!(r#"<GetDeviceInformation xmlns="{DEVICE_NS}"/>"#))
            .await?;
        Ok(DeviceInformation {
            manufacturer: text_of(&reply, "Manufacturer").unwrap_or_default(),
            model: text_of(&reply, "Model").unwrap_or_default(),
            firmware: text_of(&reply, "FirmwareVersion").unwrap_or_default(),
            serial: text_of(&reply, "SerialNumber").unwrap_or_default(),
            hardware: text_of(&reply, "HardwareId").unwrap_or_default(),
        })
    }

    pub async fn profiles(&self) -> Result<Vec<Profile>, OnvifError> {
        let reply = self
            .call(format!(r#"<GetProfiles xmlns="{MEDIA_NS}"/>"#))
            .await?;
        let block = block_of(&reply, "GetProfilesResponse").unwrap_or(&reply);
        let mut profiles = Vec::new();
        let mut cursor = 0;
        while let Some(offset) = block[cursor..].find('<') {
            let start = cursor + offset;
            let rest = &block[start + 1..];
            if rest.starts_with('/') {
                cursor = start + 1;
                continue;
            }
            let Some(end) = rest.find('>') else { break };
            let tag = &rest[..end];
            let name_end = tag
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(tag.len());
            if local_name(&tag[..name_end]) != "Profiles" {
                cursor = start + 1;
                continue;
            }
            let body_start = start + 1 + end + 1;
            let closing = format!("</{}", &tag[..name_end]);
            let Some(body_end) = block[body_start..].find(&closing) else {
                break;
            };
            let body = &block[body_start..body_start + body_end];
            profiles.push(Profile {
                token: attribute(tag, "token").unwrap_or_default(),
                name: text_of(body, "Name").unwrap_or_default(),
                video_source: element_token(body, "VideoSourceConfiguration"),
                encoder: element_token(body, "VideoEncoderConfiguration"),
            });
            cursor = body_start + body_end;
        }
        if profiles.is_empty() {
            return Err(OnvifError::Missing {
                what: "a media profile",
            });
        }
        Ok(profiles)
    }

    pub async fn ptz_node(&self) -> Result<PtzNode, OnvifError> {
        let reply = self
            .call(format!(r#"<GetNodes xmlns="{PTZ_NS}"/>"#))
            .await?;
        let node = block_of(&reply, "PTZNode").ok_or(OnvifError::Missing {
            what: "a PTZ node (this camera may have no motor)",
        })?;
        let mut auxiliary_commands = Vec::new();
        if let Some(list) = block_of(node, "AuxiliaryCommands") {
            for segment in list.split(',') {
                let trimmed = segment.trim();
                if !trimmed.is_empty() {
                    auxiliary_commands.push(trimmed.to_string());
                }
            }
        }
        Ok(PtzNode {
            token: attribute(node, "token").unwrap_or_default(),
            max_presets: number_of(node, "MaximumNumberOfPresets").unwrap_or(0),
            home_supported: text_of(node, "HomeSupported")
                .map(|value| value == "true")
                .unwrap_or(false),
            auxiliary_commands,
        })
    }

    pub async fn presets(&self) -> Result<Vec<Preset>, OnvifError> {
        let reply = self
            .call(format!(
                r#"<GetPresets xmlns="{PTZ_NS}"><ProfileToken>{}</ProfileToken></GetPresets>"#,
                self.profile
            ))
            .await?;
        Ok(segments(&reply, "Preset token=")
            .into_iter()
            .filter_map(|segment| {
                Some(Preset {
                    token: quoted(segment)?,
                    name: text_of(segment, "Name").unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn continuous_move(
        &self,
        pan: f32,
        tilt: f32,
        zoom: f32,
        hold: Duration,
    ) -> Result<(), OnvifError> {
        let seconds = hold.as_millis() as f64 / 1000.0;
        let body = format!(
            r#"<tptz:ContinuousMove xmlns:tptz="{PTZ_NS}"><tptz:ProfileToken>{profile}</tptz:ProfileToken><tptz:Velocity><tt:PanTilt x="{pan}" y="{tilt}" xmlns:tt="{SCHEMA_NS}"/><tt:Zoom x="{zoom}" xmlns:tt="{SCHEMA_NS}"/></tptz:Velocity><tptz:Timeout>PT{seconds}S</tptz:Timeout></tptz:ContinuousMove>"#,
            profile = self.profile,
        );
        self.call(body).await.map(|_| ())
    }

    pub async fn stop(&self) -> Result<(), OnvifError> {
        let body = format!(
            r#"<tptz:Stop xmlns:tptz="{PTZ_NS}"><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PanTilt>true</tptz:PanTilt><tptz:Zoom>true</tptz:Zoom></tptz:Stop>"#,
            self.profile
        );
        self.call(body).await.map(|_| ())
    }

    pub async fn goto_preset(&self, token: &str) -> Result<(), OnvifError> {
        let body = format!(
            r#"<tptz:GotoPreset xmlns:tptz="{PTZ_NS}"><tptz:ProfileToken>{}</tptz:ProfileToken><tptz:PresetToken>{token}</tptz:PresetToken></tptz:GotoPreset>"#,
            self.profile
        );
        let mut last = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
            match self.call(body.clone()).await {
                Ok(_) => return Ok(()),
                Err(OnvifError::Transport(detail)) => last = Some(OnvifError::Transport(detail)),
                Err(other) => return Err(other),
            }
        }
        Err(last.unwrap_or(OnvifError::Transport("no attempt was made".to_string())))
    }

    pub async fn set_preset(&self, name: Option<&str>) -> Result<String, OnvifError> {
        let name_xml = name
            .map(|name| format!("<tptz:PresetName>{name}</tptz:PresetName>"))
            .unwrap_or_default();
        let body = format!(
            r#"<tptz:SetPreset xmlns:tptz="{PTZ_NS}"><tptz:ProfileToken>{}</tptz:ProfileToken>{name_xml}</tptz:SetPreset>"#,
            self.profile
        );
        let reply = self.call(body).await?;
        text_of(&reply, "PresetToken").ok_or(OnvifError::Missing {
            what: "the new preset token",
        })
    }

    pub fn relative_move(&self, _pan: f32, _tilt: f32, _zoom: f32) -> Result<(), OnvifError> {
        Err(OnvifError::Blocked {
            operation: "RelativeMove",
            reason: "on this firmware the call takes every device service down for 35-45 s (ports 80, 554, 8899 and 34567 stop accepting connections); use ContinuousMove with an explicit Timeout instead",
        })
    }

    pub async fn ptz_status(&self) -> Result<(), OnvifError> {
        let reply = self
            .call(format!(
                r#"<tptz:GetStatus xmlns:tptz="{PTZ_NS}"><tptz:ProfileToken>{}</tptz:ProfileToken></tptz:GetStatus>"#,
                self.profile
            ))
            .await?;
        let _ = reply;
        Err(OnvifError::Blocked {
            operation: "GetStatus",
            reason: "the firmware answers ter:ActionNotSupported; the logical position has to be tracked by the application and restored with GotoPreset",
        })
    }

    pub async fn encoder_configuration(
        &self,
        token: &str,
    ) -> Result<EncoderConfiguration, OnvifError> {
        let body = format!(
            r#"<trt:GetVideoEncoderConfiguration xmlns:trt="{MEDIA_NS}"><trt:ConfigurationToken>{token}</trt:ConfigurationToken></trt:GetVideoEncoderConfiguration>"#
        );
        let reply = self.call(body).await?;
        let config = block_of(&reply, "Configuration").ok_or(OnvifError::Missing {
            what: "a video encoder configuration",
        })?;
        let resolution = block_of(config, "Resolution").unwrap_or("");
        Ok(EncoderConfiguration {
            token: attribute(config, "token").unwrap_or_else(|| token.to_string()),
            name: text_of(config, "Name").unwrap_or_default(),
            encoding: text_of(config, "Encoding").unwrap_or_default(),
            width: number_of(resolution, "Width").unwrap_or(0),
            height: number_of(resolution, "Height").unwrap_or(0),
            frame_rate: number_of(config, "FrameRateLimit").unwrap_or(0),
            bitrate_kbps: number_of(config, "BitrateLimit").unwrap_or(0),
            gop: number_of(config, "GovLength").unwrap_or(0),
        })
    }

    pub async fn imaging_settings(
        &self,
        video_source: &str,
    ) -> Result<serde_json::Value, OnvifError> {
        let body = format!(
            r#"<timg:GetImagingSettings xmlns:timg="{IMAGING_NS}"><timg:VideoSourceToken>{video_source}</timg:VideoSourceToken></timg:GetImagingSettings>"#
        );
        let reply = self.call(body).await?;
        let settings = block_of(&reply, "ImagingSettings").ok_or(OnvifError::Missing {
            what: "imaging settings",
        })?;
        let mut map = serde_json::Map::new();
        for (key, element) in [
            ("brightness", "Brightness"),
            ("color_saturation", "ColorSaturation"),
            ("contrast", "Contrast"),
            ("sharpness", "Sharpness"),
            ("ir_cut_filter", "IrCutFilter"),
            ("backlight_compensation_mode", "BacklightCompensationMode"),
            ("exposure_mode", "ExposureMode"),
        ] {
            if let Some(value) = text_of(settings, element) {
                map.insert(key.to_string(), serde_json::Value::String(value));
            }
        }
        Ok(serde_json::Value::Object(map))
    }

    pub async fn network_interfaces(&self) -> Result<Vec<NetworkInterface>, OnvifError> {
        let reply = self
            .call(format!(r#"<GetNetworkInterfaces xmlns="{DEVICE_NS}"/>"#))
            .await?;
        Ok(segments(&reply, "NetworkInterfaces token=")
            .into_iter()
            .filter_map(|segment| {
                Some(NetworkInterface {
                    token: quoted(segment)?,
                    name: text_of(segment, "Name").unwrap_or_default(),
                    enabled: text_of(segment, "Enabled")
                        .map(|value| value == "true")
                        .unwrap_or(false),
                    ipv4: text_of(segment, "Address").into_iter().collect(),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE_INFO: &str = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body><tds:GetDeviceInformationResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl"><tds:Manufacturer>H264</tds:Manufacturer><tds:Model>XM535_X6E-WEQ_8M</tds:Model><tds:FirmwareVersion>V5.01.R02.G51A07F3.10010.346732.S.ONVIF 21.06</tds:FirmwareVersion><tds:SerialNumber>0123456789abcdef0123</tds:SerialNumber><tds:HardwareId>00001</tds:HardwareId></tds:GetDeviceInformationResponse></s:Body></s:Envelope>"#;

    const PROFILES: &str = r#"<trt:GetProfilesResponse xmlns:trt="http://www.onvif.org/ver10/media/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><trt:Profiles fixed="true" token="000"><tt:Name>mainStream</tt:Name><tt:VideoSourceConfiguration token="V_SRC_000"><tt:Name>V_SRC_CFG_000</tt:Name><tt:SourceToken>V_SRC_000</tt:SourceToken></tt:VideoSourceConfiguration><tt:VideoEncoderConfiguration token="000"><tt:Name>V_ENC_000</tt:Name><tt:Encoding>H264</tt:Encoding><tt:Resolution><tt:Width>3072</tt:Width><tt:Height>2048</tt:Height></tt:Resolution></tt:VideoEncoderConfiguration></trt:Profiles></trt:GetProfilesResponse>"#;

    const PRESETS: &str = r#"<tptz:GetPresetsResponse xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><tptz:Preset token="0"><tt:Name>0</tt:Name></tptz:Preset><tptz:Preset token="1"><tt:Name>1</tt:Name></tptz:Preset></tptz:GetPresetsResponse>"#;

    const NODE: &str = r#"<tptz:GetNodesResponse xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><tptz:PTZNode token="NODE_000"><tt:Name>PTZNode</tt:Name><tt:SupportedPTZSpaces><tt:PanTiltSpeedSpace><tt:XRange><tt:Min>-1.0</tt:Min><tt:Max>1.0</tt:Max></tt:XRange></tt:PanTiltSpeedSpace></tt:SupportedPTZSpaces><tt:MaximumNumberOfPresets>128</tt:MaximumNumberOfPresets><tt:HomeSupported>true</tt:HomeSupported><tt:AuxiliaryCommands>Wiper start,Wiper stop</tt:AuxiliaryCommands></tptz:PTZNode></tptz:GetNodesResponse>"#;

    const ENCODER: &str = r#"<trt:GetVideoEncoderConfigurationResponse xmlns:trt="http://www.onvif.org/ver10/media/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><trt:Configuration token="000"><tt:Name>V_ENC_000</tt:Name><tt:Encoding>H264</tt:Encoding><tt:Resolution><tt:Width>3072</tt:Width><tt:Height>2048</tt:Height></tt:Resolution><tt:Quality>3</tt:Quality><tt:RateControl><tt:FrameRateLimit>12</tt:FrameRateLimit><tt:BitrateLimit>1536</tt:BitrateLimit></tt:RateControl><tt:H264><tt:GovLength>36</tt:GovLength></tt:H264></trt:Configuration></trt:GetVideoEncoderConfigurationResponse>"#;

    const IMAGING: &str = r#"<timg:GetImagingSettingsResponse xmlns:timg="http://www.onvif.org/ver20/imaging/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><timg:ImagingSettings><tt:Brightness>50</tt:Brightness><tt:IrCutFilter>OFF</tt:IrCutFilter><tt:Exposure><tt:ExposureMode>AUTO</tt:ExposureMode></tt:Exposure></timg:ImagingSettings></timg:GetImagingSettingsResponse>"#;

    #[test]
    fn device_information_is_read_from_the_namespaced_reply() {
        assert_eq!(
            text_of(DEVICE_INFO, "Model").as_deref(),
            Some("XM535_X6E-WEQ_8M")
        );
        assert_eq!(
            text_of(DEVICE_INFO, "FirmwareVersion").as_deref(),
            Some("V5.01.R02.G51A07F3.10010.346732.S.ONVIF 21.06")
        );
        assert_eq!(
            text_of(DEVICE_INFO, "SerialNumber").as_deref(),
            Some("0123456789abcdef0123")
        );
    }

    #[test]
    fn profile_tokens_come_from_the_profiles_block_not_from_children() {
        let block = block_of(PROFILES, "Profiles").unwrap();
        assert_eq!(text_of(block, "Name").as_deref(), Some("mainStream"));
        let profile_token =
            quoted(segments(PROFILES, "Profiles fixed=\"true\" token=")[0]).unwrap();
        assert_eq!(profile_token, "000");
        let source_token =
            quoted(segments(PROFILES, "VideoSourceConfiguration token=")[0]).unwrap();
        assert_eq!(source_token, "V_SRC_000");
        let encoder_token =
            quoted(segments(PROFILES, "VideoEncoderConfiguration token=")[0]).unwrap();
        assert_eq!(encoder_token, "000");
        assert!(block.contains("SourceToken>V_SRC_000<"));
        assert!(!segments(PROFILES, "VideoEncoderConfiguration token=")[0].contains("V_SRC_000"));
    }

    #[test]
    fn preset_segments_carry_a_token_and_a_name() {
        let rows: Vec<(String, String)> = segments(PRESETS, "Preset token=")
            .into_iter()
            .filter_map(|segment| Some((quoted(segment)?, text_of(segment, "Name")?)))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("0".to_string(), "0".to_string()),
                ("1".to_string(), "1".to_string())
            ]
        );
    }

    #[test]
    fn ptz_node_reports_preset_limit_and_auxiliary_commands() {
        let node = block_of(NODE, "PTZNode").unwrap();
        assert_eq!(
            quoted(segments(NODE, "PTZNode token=")[0]).unwrap(),
            "NODE_000"
        );
        assert_eq!(number_of(node, "MaximumNumberOfPresets"), Some(128));
        assert_eq!(text_of(node, "HomeSupported").as_deref(), Some("true"));
        assert_eq!(
            block_of(node, "AuxiliaryCommands").map(str::trim),
            Some("Wiper start,Wiper stop")
        );
    }

    #[test]
    fn encoder_configuration_exposes_resolution_and_rate() {
        let config = block_of(ENCODER, "Configuration").unwrap();
        let resolution = block_of(config, "Resolution").unwrap();
        assert_eq!(number_of(resolution, "Width"), Some(3072));
        assert_eq!(number_of(resolution, "Height"), Some(2048));
        assert_eq!(number_of(config, "FrameRateLimit"), Some(12));
        assert_eq!(number_of(config, "GovLength"), Some(36));
        assert_eq!(text_of(config, "Encoding").as_deref(), Some("H264"));
    }

    #[test]
    fn imaging_settings_flatten_into_named_values() {
        let settings = block_of(IMAGING, "ImagingSettings").unwrap();
        assert_eq!(text_of(settings, "Brightness").as_deref(), Some("50"));
        assert_eq!(text_of(settings, "IrCutFilter").as_deref(), Some("OFF"));
        assert_eq!(text_of(settings, "ExposureMode").as_deref(), Some("AUTO"));
    }

    #[test]
    fn faults_carry_the_device_code_and_reason() {
        let error = fault(
            r#"<s:Fault><s:Code><s:Value>s:Sender</s:Value><s:Subcode><s:Value>ter:ActionNotSupported</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang="en">Action Not Implemented</s:Text></s:Reason></s:Fault>"#,
            400,
        );
        match error {
            OnvifError::Fault {
                status,
                code,
                reason,
            } => {
                assert_eq!(status, 400);
                assert_eq!(code, "s:Sender");
                assert_eq!(reason, "Action Not Implemented");
            }
            other => panic!("expected fault, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn relative_move_is_refused_before_any_request_is_sent() {
        let client = Client::new("http://127.0.0.1:1/onvif/device_service");
        match client.relative_move(0.5, 0.0, 0.0) {
            Err(OnvifError::Blocked { operation, reason }) => {
                assert_eq!(operation, "RelativeMove");
                assert!(reason.contains("35-45"));
            }
            other => panic!("expected a block, got {other:?}"),
        }
    }
}
