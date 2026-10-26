use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proview_core::live::{plan_clip, ClipPlan, LiveHub, StreamKey};
use proview_core::media::fmp4::{avcc_sample, Sample, TrackConfig, TIMESCALE};
use proview_core::media::h264::parse_sps;
use proview_core::media::nal::split_annexb;
use proview_core::source::RtspTarget;

struct SourceEntry {
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct LiveState {
    pub hub: Arc<LiveHub>,
    plans: Arc<Mutex<HashMap<String, Arc<ClipPlan>>>>,
    sources: Arc<Mutex<HashMap<StreamKey, SourceEntry>>>,
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
            sources: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn ensure_rtsp_source(&self, key: StreamKey, target: RtspTarget) -> bool {
        let mut guard = self.sources.lock().expect("sources poisoned");
        if let Some(entry) = guard.get(&key) {
            if entry.alive.load(Ordering::Relaxed) {
                return false;
            }
            guard.remove(&key);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        guard.insert(
            key.clone(),
            SourceEntry {
                stop: stop.clone(),
                alive: alive.clone(),
            },
        );
        drop(guard);

        let pump_hub = self.hub.clone();
        let finish = self.sources.clone();
        let running = key.clone();
        let pump_stop = stop.clone();
        tokio::spawn(async move {
            proview_core::source::pump(pump_hub, running.clone(), target, pump_stop).await;
            alive.store(false, Ordering::Relaxed);
            finish.lock().expect("sources poisoned").remove(&running);
        });

        let sources = self.sources.clone();
        let watcher_hub = self.hub.clone();
        let watcher_key = key.clone();
        tokio::spawn(async move {
            let mut idle_since: Option<Instant> = None;
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if watcher_hub.subscriber_count(&watcher_key) == 0 {
                    let since = *idle_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= Duration::from_secs(60) {
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                } else {
                    idle_since = None;
                }
            }
            sources
                .lock()
                .expect("sources poisoned")
                .remove(&watcher_key);
        });
        true
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
        if self.hub.has_publisher(&key) {
            return;
        }
        proview_core::live::spawn_publisher(self.hub.clone(), key, plan.init.clone(), plan);
    }

    pub fn stop_source(&self, key: &StreamKey) -> bool {
        let guard = self.sources.lock().expect("sources poisoned");
        match guard.get(key) {
            Some(entry) => {
                entry.stop.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
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
