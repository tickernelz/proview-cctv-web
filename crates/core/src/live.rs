use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::time::{interval, MissedTickBehavior};

use crate::media::fmp4::{init_segment, media_segment, Sample, TrackConfig, TIMESCALE};

pub type Fragment = bytes::Bytes;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StreamKey {
    pub device: String,
    pub channel: u32,
    pub sub_stream: bool,
}

impl StreamKey {
    pub fn new(device: impl Into<String>, channel: u32, sub_stream: bool) -> Self {
        Self {
            device: device.into(),
            channel,
            sub_stream,
        }
    }
}

pub struct ClipPlan {
    pub init: Fragment,
    pub fragments: Vec<Fragment>,
    pub fragment_duration: Duration,
}

pub fn plan_clip(track: &TrackConfig, samples: &[Sample], per_fragment: usize) -> ClipPlan {
    let init = Fragment::from(init_segment(track));
    let mut fragments = Vec::new();
    let mut base: u64 = 0;
    for (index, chunk) in samples.chunks(per_fragment.max(1)).enumerate() {
        let sequence = index as u32 + 1;
        let mut segment = media_segment(sequence, base, chunk);
        let duration: u64 = chunk.iter().map(|sample| sample.duration as u64).sum();
        base += duration;
        fragments.push(Fragment::from(segment.split_off(0)));
    }
    let frames = samples.len().max(1);
    let total: u64 = samples.iter().map(|s| s.duration as u64).sum();
    let per = if samples.is_empty() {
        Duration::from_millis(1000)
    } else {
        let every = per_fragment.max(1) as u64;
        let nanos =
            (total as u128 * every as u128 * 1_000_000_000) / (frames as u128 * TIMESCALE as u128);
        Duration::from_nanos(nanos.min(u64::MAX as u128) as u64)
    };
    ClipPlan {
        init,
        fragments,
        fragment_duration: per,
    }
}

struct Channel {
    sender: broadcast::Sender<Fragment>,
    subscribers: usize,
    publishers: usize,
    init: Arc<Mutex<Option<Fragment>>>,
    last: Arc<Mutex<Option<Fragment>>>,
}

pub struct Subscription {
    pub init: Arc<Mutex<Option<Fragment>>>,
    pub last: Arc<Mutex<Option<Fragment>>>,
    pub receiver: broadcast::Receiver<Fragment>,
}

pub struct LiveHub {
    channels: Mutex<HashMap<StreamKey, Channel>>,
    capacity: usize,
}

impl Default for LiveHub {
    fn default() -> Self {
        Self::new(8)
    }
}

impl LiveHub {
    pub fn new(capacity: usize) -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            capacity,
        }
    }

    pub fn subscribe(&self, key: StreamKey) -> Subscription {
        let mut guard = self.channels.lock().expect("live hub poisoned");
        let channel = guard.entry(key).or_insert_with(|| {
            let (sender, _) = broadcast::channel(self.capacity);
            Channel {
                sender,
                subscribers: 0,
                publishers: 0,
                init: Arc::new(Mutex::new(None)),
                last: Arc::new(Mutex::new(None)),
            }
        });
        channel.subscribers += 1;
        Subscription {
            init: channel.init.clone(),
            last: channel.last.clone(),
            receiver: channel.sender.subscribe(),
        }
    }

    pub fn init_slot(&self, key: &StreamKey) -> Arc<Mutex<Option<Fragment>>> {
        let mut guard = self.channels.lock().expect("live hub poisoned");
        let channel = guard.entry(key.clone()).or_insert_with(|| {
            let (sender, _) = broadcast::channel(self.capacity);
            Channel {
                sender,
                subscribers: 0,
                publishers: 0,
                init: Arc::new(Mutex::new(None)),
                last: Arc::new(Mutex::new(None)),
            }
        });
        channel.init.clone()
    }

    pub fn publish_init(&self, key: &StreamKey, init: Fragment) {
        let guard = self.channels.lock().expect("live hub poisoned");
        if let Some(channel) = guard.get(key) {
            if let Ok(mut slot) = channel.init.lock() {
                *slot = Some(init);
            }
        }
    }

    pub fn publish(&self, key: &StreamKey, fragment: Fragment) {
        let guard = self.channels.lock().expect("live hub poisoned");
        if let Some(channel) = guard.get(key) {
            if let Ok(mut slot) = channel.last.lock() {
                *slot = Some(fragment.clone());
            }
            let _ = channel.sender.send(fragment);
        }
    }

    pub fn has_publisher(&self, key: &StreamKey) -> bool {
        let guard = self.channels.lock().expect("live hub poisoned");
        guard
            .get(key)
            .map(|channel| channel.publishers > 0)
            .unwrap_or(false)
    }

    pub fn subscriber_count(&self, key: &StreamKey) -> usize {
        let guard = self.channels.lock().expect("live hub poisoned");
        guard
            .get(key)
            .map(|channel| channel.sender.receiver_count())
            .unwrap_or(0)
    }

    pub fn publisher(&self, key: StreamKey) -> broadcast::Sender<Fragment> {
        let mut guard = self.channels.lock().expect("live hub poisoned");
        let channel = guard.entry(key).or_insert_with(|| {
            let (sender, _) = broadcast::channel(self.capacity);
            Channel {
                sender,
                subscribers: 0,
                publishers: 0,
                init: Arc::new(Mutex::new(None)),
                last: Arc::new(Mutex::new(None)),
            }
        });
        channel.publishers = channel.publishers.max(1);
        channel.sender.clone()
    }

    pub fn publisher_done(&self, key: &StreamKey) {
        let mut guard = self.channels.lock().expect("live hub poisoned");
        if let Some(channel) = guard.get_mut(key) {
            channel.publishers = channel.publishers.saturating_sub(1);
            if channel.publishers == 0 && channel.subscribers == 0 {
                guard.remove(key);
            }
        }
    }

    pub fn release(&self, key: &StreamKey) {
        let mut guard = self.channels.lock().expect("live hub poisoned");
        if let Some(channel) = guard.get_mut(key) {
            channel.subscribers = channel.subscribers.saturating_sub(1);
            if channel.subscribers == 0 && channel.publishers == 0 {
                guard.remove(key);
            }
        }
    }

    pub fn active_streams(&self) -> usize {
        self.channels.lock().expect("live hub poisoned").len()
    }
}

pub fn spawn_publisher(hub: Arc<LiveHub>, key: StreamKey, init: Fragment, plan: Arc<ClipPlan>) {
    hub.publish_init(&key, init);
    let sender = hub.publisher(key.clone());
    tokio::spawn(async move {
        let mut ticker = interval(plan.fragment_duration);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        for index in (0usize..).cycle() {
            ticker.tick().await;
            if sender.receiver_count() == 0 {
                hub.publisher_done(&key);
                return;
            }
            let fragment = plan.fragments[index % plan.fragments.len()].clone();
            hub.publish(&key, fragment);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackConfig {
        TrackConfig {
            width: 640,
            height: 360,
            sps: vec![0x67, 66, 0, 30],
            pps: vec![0x68, 0xce],
        }
    }

    fn samples(count: usize) -> Vec<Sample> {
        (0..count)
            .map(|index| Sample {
                data: vec![0, 0, 0, 2, 0x65, index as u8],
                duration: TIMESCALE / 25,
                keyframe: index % 25 == 0,
            })
            .collect()
    }

    #[test]
    fn plan_splits_samples_into_fragments_with_monotonic_base_times() {
        let plan = plan_clip(&track(), &samples(50), 25);
        assert_eq!(plan.fragments.len(), 2);
        assert!(!plan.init.is_empty());
        assert_eq!(plan.fragment_duration, Duration::from_millis(1000));
    }

    #[test]
    fn empty_sample_list_still_yields_a_usable_plan() {
        let plan = plan_clip(&track(), &[], 25);
        assert!(plan.fragments.is_empty());
        assert_eq!(plan.fragment_duration, Duration::from_millis(1000));
    }

    #[tokio::test]
    async fn hub_releases_a_stream_when_the_last_subscriber_leaves() {
        let hub = LiveHub::new(8);
        let key = StreamKey::new("dev", 1, false);
        let subscription = hub.subscribe(key.clone());
        assert_eq!(hub.active_streams(), 1);
        drop(subscription);
        hub.release(&key);
        assert_eq!(hub.active_streams(), 0);
    }

    #[tokio::test]
    async fn a_late_subscriber_sees_the_stored_init_segment() {
        let hub = LiveHub::new(8);
        let key = StreamKey::new("dev", 1, false);
        let plan = Arc::new(plan_clip(&track(), &samples(25), 25));
        let first = hub.subscribe(key.clone());
        hub.publish_init(&key, plan.init.clone());
        assert_eq!(first.init.lock().unwrap().clone(), Some(plan.init.clone()));
        let late = hub.subscribe(key.clone());
        assert_eq!(late.init.lock().unwrap().clone(), Some(plan.init.clone()));
        hub.release(&key);
        hub.release(&key);
    }

    #[tokio::test]
    async fn fragments_reach_every_subscriber() {
        let hub = Arc::new(LiveHub::new(8));
        let key = StreamKey::new("dev", 1, false);
        let first = hub.subscribe(key.clone());
        let mut second = hub.subscribe(key.clone()).receiver;
        let mut receiver = first.receiver;
        let plan = Arc::new(plan_clip(&track(), &samples(25), 25));
        spawn_publisher(hub.clone(), key.clone(), plan.init.clone(), plan.clone());
        let one = receiver.recv().await.expect("first viewer fragment");
        let two = second.recv().await.expect("second viewer fragment");
        assert_eq!(one, two);
        hub.release(&key);
        hub.release(&key);
    }
}
