use serde::{Deserialize, Serialize};

use crate::lens::{
    detect_layout, lens_label, stacked_crops, FrameSize, FrameSizeSource, LensCapabilities,
    LensCountSource, LensCrop, LensStack,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtzSource {
    MeasuredDefault,
    Operator,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensStreamInfo {
    pub quality: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub size_source: FrameSizeSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensEntry {
    pub index: u32,
    pub label: String,
    pub crop: LensCrop,
    pub ptz: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensView {
    pub device_id: String,
    pub channel: u32,
    pub streams: Vec<LensStreamInfo>,
    pub lenses: Vec<LensEntry>,
    pub lens_count: u32,
    pub count_source: LensCountSource,
    pub stack: LensStack,
    pub crops_verified: bool,
    pub capabilities: LensCapabilities,
    pub ptz_source: PtzSource,
}

pub struct LensViewRequest<'a> {
    pub device_id: &'a str,
    pub channel: u32,
    pub streams: Vec<LensStreamInfo>,
    pub capabilities: LensCapabilities,
    pub ptz: Option<u32>,
}

fn primary_frame(streams: &[LensStreamInfo]) -> FrameSize {
    streams
        .iter()
        .find(|stream| stream.quality == "main")
        .or_else(|| streams.first())
        .map(|stream| FrameSize {
            width: stream.width,
            height: stream.height,
        })
        .unwrap_or(FrameSize {
            width: 0,
            height: 0,
        })
}

fn measured_ptz_lens(count: u32) -> Option<u32> {
    if count == 2 {
        Some(1)
    } else {
        None
    }
}

impl LensView {
    pub fn build(request: LensViewRequest<'_>) -> Self {
        let layout = detect_layout(primary_frame(&request.streams), request.capabilities);
        let ptz = request.ptz.or_else(|| measured_ptz_lens(layout.count));
        let ptz_source = if request.ptz.is_some() {
            PtzSource::Operator
        } else if ptz.is_some() {
            PtzSource::MeasuredDefault
        } else {
            PtzSource::Unknown
        };
        let lenses = if layout.count == 0 {
            Vec::new()
        } else {
            stacked_crops(&layout)
                .into_iter()
                .enumerate()
                .map(|(index, crop)| {
                    let index = index as u32;
                    LensEntry {
                        index,
                        label: lens_label(index, layout.count).to_string(),
                        crop,
                        ptz: ptz == Some(index),
                    }
                })
                .collect()
        };
        Self {
            device_id: request.device_id.to_string(),
            channel: request.channel,
            streams: request.streams,
            lenses,
            lens_count: layout.count,
            count_source: layout.source,
            stack: layout.stack,
            crops_verified: layout.crops_verified(),
            capabilities: request.capabilities,
            ptz_source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn measured_streams() -> Vec<LensStreamInfo> {
        vec![
            LensStreamInfo {
                quality: "main".to_string(),
                codec: "H265".to_string(),
                width: 2304,
                height: 2592,
                size_source: FrameSizeSource::SdpSps,
            },
            LensStreamInfo {
                quality: "sub".to_string(),
                codec: "H265".to_string(),
                width: 640,
                height: 720,
                size_source: FrameSizeSource::SdpSps,
            },
        ]
    }

    #[test]
    fn the_measured_device_yields_two_stacked_lenses() {
        let view = LensView::build(LensViewRequest {
            device_id: "fixture-dev-0001",
            channel: 1,
            streams: measured_streams(),
            capabilities: LensCapabilities::default(),
            ptz: None,
        });
        assert_eq!(view.lens_count, 2);
        assert_eq!(view.count_source, LensCountSource::Geometry);
        assert_eq!(view.stack, LensStack::Vertical);
        assert!(view.crops_verified);
        assert_eq!(view.lenses.len(), 2);
        assert_eq!(view.lenses[0].label, "top");
        assert_eq!(view.lenses[1].label, "bottom");
        assert_eq!(
            view.lenses[1].crop,
            LensCrop {
                x: 0.0,
                y: 0.5,
                w: 1.0,
                h: 0.5
            }
        );
        assert!(!view.lenses[0].ptz);
        assert!(view.lenses[1].ptz);
        assert_eq!(view.ptz_source, PtzSource::MeasuredDefault);
    }

    #[test]
    fn an_operator_override_declares_the_ptz_lens() {
        let view = LensView::build(LensViewRequest {
            device_id: "fixture-dev-0001",
            channel: 1,
            streams: measured_streams(),
            capabilities: LensCapabilities::default(),
            ptz: Some(0),
        });
        assert_eq!(view.ptz_source, PtzSource::Operator);
        assert!(view.lenses[0].ptz);
        assert!(!view.lenses[1].ptz);
    }

    #[test]
    fn an_unmeasured_device_yields_no_lenses() {
        let view = LensView::build(LensViewRequest {
            device_id: "fixture-dev-0001",
            channel: 1,
            streams: vec![LensStreamInfo {
                quality: "main".to_string(),
                codec: "H264".to_string(),
                width: 0,
                height: 0,
                size_source: FrameSizeSource::Unknown,
            }],
            capabilities: LensCapabilities::default(),
            ptz: None,
        });
        assert_eq!(view.lens_count, 0);
        assert_eq!(view.count_source, LensCountSource::Unknown);
        assert!(view.lenses.is_empty());
        assert!(!view.crops_verified);
        assert_eq!(view.ptz_source, PtzSource::Unknown);
    }

    #[test]
    fn a_single_lens_view_claims_no_ptz() {
        let view = LensView::build(LensViewRequest {
            device_id: "fixture-dev-0001",
            channel: 1,
            streams: vec![LensStreamInfo {
                quality: "main".to_string(),
                codec: "H264".to_string(),
                width: 640,
                height: 360,
                size_source: FrameSizeSource::SdpSps,
            }],
            capabilities: LensCapabilities::default(),
            ptz: None,
        });
        assert_eq!(view.lens_count, 1);
        assert_eq!(view.lenses.len(), 1);
        assert!(!view.lenses[0].ptz);
        assert_eq!(view.ptz_source, PtzSource::Unknown);
    }

    #[test]
    fn the_wire_shape_matches_the_documented_contract() {
        let view = LensView::build(LensViewRequest {
            device_id: "fixture-dev-0001",
            channel: 1,
            streams: measured_streams(),
            capabilities: LensCapabilities {
                multi_lens_two_sensor: true,
                support_gun_ball_two_sensor_ptz_locate: true,
                ..LensCapabilities::default()
            },
            ptz: None,
        });
        let value: Value = serde_json::to_value(&view).expect("serialises");
        assert_eq!(value["deviceId"], "fixture-dev-0001");
        assert_eq!(value["channel"], 1);
        assert_eq!(value["streams"][0]["quality"], "main");
        assert_eq!(value["streams"][0]["codec"], "H265");
        assert_eq!(value["streams"][0]["width"], 2304);
        assert_eq!(value["streams"][0]["height"], 2592);
        assert_eq!(value["streams"][0]["sizeSource"], "sdp_sps");
        assert_eq!(value["lenses"][0]["index"], 0);
        assert_eq!(value["lenses"][0]["label"], "top");
        assert_eq!(value["lenses"][0]["crop"]["x"], 0.0);
        assert_eq!(value["lenses"][0]["crop"]["y"], 0.0);
        assert_eq!(value["lenses"][0]["crop"]["w"], 1.0);
        assert_eq!(value["lenses"][0]["crop"]["h"], 0.5);
        assert_eq!(value["lenses"][0]["ptz"], false);
        assert_eq!(value["lenses"][1]["ptz"], true);
        assert_eq!(value["lensCount"], 2);
        assert_eq!(value["countSource"], "capability");
        assert_eq!(value["stack"], "vertical");
        assert_eq!(value["cropsVerified"], true);
        assert_eq!(value["capabilities"]["multiLensTwoSensor"], true);
        assert_eq!(
            value["capabilities"]["supportGunBallTwoSensorPtzLocate"],
            true
        );
        assert_eq!(value["ptzSource"], "measured_default");
    }
}
