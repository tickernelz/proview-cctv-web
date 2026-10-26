use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use proview_core::live::{plan_clip, ClipPlan, LiveHub, StreamKey};
use proview_core::media::fmp4::{avcc_sample, Sample, TrackConfig, TIMESCALE};
use proview_core::media::h264::parse_sps;
use proview_core::media::nal::split_annexb;

#[derive(Clone)]
pub struct LiveState {
    pub hub: Arc<LiveHub>,
    plans: Arc<Mutex<HashMap<String, Arc<ClipPlan>>>>,
}

impl Default for LiveState {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveState {
    pub fn new() -> Self {
        Self {
            hub: Arc::new(LiveHub::default()),
            plans: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn plan_for(&self, path: &str, per_fragment: usize) -> Result<Arc<ClipPlan>, String> {
        let mut guard = self.plans.lock().expect("plan cache poisoned");
        if let Some(plan) = guard.get(path) {
            return Ok(plan.clone());
        }
        let clip = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        let plan = Arc::new(plan_from_annexb(&clip, per_fragment)?);
        guard.insert(path.to_string(), plan.clone());
        Ok(plan)
    }

    pub fn ensure_publisher(&self, key: StreamKey, plan: Arc<ClipPlan>) {
        let sender = self.hub.publisher(key);
        if sender.receiver_count() > 0 {
            return;
        }
        proview_core::live::spawn_publisher(sender, plan);
    }

    pub fn release(&self, key: &StreamKey) {
        self.hub.release(key);
    }

    pub fn inner(&self) -> &Self {
        self
    }
}

fn plan_from_annexb(clip: &[u8], per_fragment: usize) -> Result<ClipPlan, String> {
    let units = split_annexb(clip);
    let sps = units
        .iter()
        .find(|unit| unit.h264_type() == Some(7))
        .map(|unit| unit.data.to_vec())
        .ok_or_else(|| "clip has no SPS".to_string())?;
    let pps = units
        .iter()
        .find(|unit| unit.h264_type() == Some(8))
        .map(|unit| unit.data.to_vec())
        .unwrap_or_default();
    let info = parse_sps(&sps).map_err(|e| format!("SPS rejected: {e:?}"))?;
    let track = TrackConfig {
        width: info.width.max(16),
        height: info.height.max(16),
        sps,
        pps,
    };

    let duration = TIMESCALE / 25;
    let mut samples: Vec<Sample> = Vec::new();
    let mut pending: Vec<&[u8]> = Vec::new();
    for unit in &units {
        match unit.h264_type() {
            Some(1) | Some(5) => {
                let mut data = Vec::new();
                for nal in pending.drain(..) {
                    data.extend_from_slice(&avcc_sample(nal));
                }
                data.extend_from_slice(&avcc_sample(unit.data));
                samples.push(Sample {
                    data,
                    duration,
                    keyframe: unit.h264_type() == Some(5),
                });
            }
            Some(7) | Some(8) => {}
            _ => pending.push(unit.data),
        }
    }
    if samples.is_empty() {
        return Err("clip has no frames".to_string());
    }
    Ok(plan_clip(&track, &samples, per_fragment))
}
