use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::time::{interval, MissedTickBehavior};

use crate::media::fmp4::{init_segment, media_segment, Sample, TrackConfig, TIMESCALE};

pub type Fragment = bytes::Bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiveMode {
    Transcode,
    Passthrough,
}

impl LiveMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LiveMode::Transcode => "transcode",
            LiveMode::Passthrough => "passthrough",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketKind {
    Init,
    Fragment,
    Config,
    AccessUnit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivePacket {
    pub kind: PacketKind,
    pub bytes: Fragment,
    pub keyframe: bool,
    pub pts_us: u64,
}

impl LivePacket {
    pub fn init(bytes: Fragment) -> Self {
        Self {
            kind: PacketKind::Init,
            bytes,
            keyframe: false,
            pts_us: 0,
        }
    }

    pub fn fragment(bytes: Fragment) -> Self {
        Self {
            kind: PacketKind::Fragment,
            bytes,
            keyframe: true,
            pts_us: 0,
        }
    }

    pub fn config(bytes: Fragment) -> Self {
        Self {
            kind: PacketKind::Config,
            bytes,
            keyframe: true,
            pts_us: 0,
        }
    }

    pub fn access_unit(bytes: Fragment, keyframe: bool, pts_us: u64) -> Self {
        Self {
            kind: PacketKind::AccessUnit,
            bytes,
            keyframe,
            pts_us,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamMeta {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_clock: Option<u32>,
    pub codec_string: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Default)]
pub struct Counters {
    pub packets: AtomicU64,
    pub keyframes: AtomicU64,
    pub bytes: AtomicU64,
    pub audio_packets: AtomicU64,
    pub drops: AtomicU64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSnapshot {
    pub device: String,
    pub channel: u32,
    pub sub_stream: bool,
    pub mode: LiveMode,
    pub subscribers: usize,
    pub publishers: usize,
    pub packets: u64,
    pub keyframes: u64,
    pub bytes: u64,
    pub audio_packets: u64,
    pub drops: u64,
    pub meta: StreamMeta,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StreamKey {
    pub scope: String,
    pub device: String,
    pub channel: u32,
    pub sub_stream: bool,
    pub mode: LiveMode,
}

impl StreamKey {
    pub fn new(device: impl Into<String>, channel: u32, sub_stream: bool, mode: LiveMode) -> Self {
        Self {
            scope: String::new(),
            device: device.into(),
            channel,
            sub_stream,
            mode,
        }
    }

    pub fn cloud(
        scope: impl Into<String>,
        device: impl Into<String>,
        channel: u32,
        sub_stream: bool,
        mode: LiveMode,
    ) -> Self {
        Self {
            scope: scope.into(),
            device: device.into(),
            channel,
            sub_stream,
            mode,
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
    sender: broadcast::Sender<Arc<LivePacket>>,
    subscribers: usize,
    publishers: usize,
    init: Arc<Mutex<Option<Fragment>>>,
    config: Arc<Mutex<Option<Fragment>>>,
    last: Arc<Mutex<Option<Arc<LivePacket>>>>,
    meta: Arc<Mutex<StreamMeta>>,
    counters: Arc<Counters>,
}

impl Channel {
    fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            subscribers: 0,
            publishers: 0,
            init: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(None)),
            last: Arc::new(Mutex::new(None)),
            meta: Arc::new(Mutex::new(StreamMeta::default())),
            counters: Arc::new(Counters::default()),
        }
    }
}

pub struct Subscription {
    pub init: Arc<Mutex<Option<Fragment>>>,
    pub config: Arc<Mutex<Option<Fragment>>>,
    pub last: Arc<Mutex<Option<Arc<LivePacket>>>>,
    pub meta: Arc<Mutex<StreamMeta>>,
    pub receiver: broadcast::Receiver<Arc<LivePacket>>,
}

const PASSTHROUGH_CAPACITY: usize = 96;

pub struct LiveHub {
    channels: Mutex<HashMap<StreamKey, Channel>>,
    capacity: usize,
}

impl LiveHub {
    fn capacity_for(&self, mode: LiveMode) -> usize {
        match mode {
            LiveMode::Passthrough => PASSTHROUGH_CAPACITY.max(self.capacity),
            LiveMode::Transcode => self.capacity,
        }
    }
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
        let capacity = self.capacity_for(key.mode);
        let mut guard = self.channels.lock().expect("live hub poisoned");
        let channel = guard.entry(key).or_insert_with(|| Channel::new(capacity));
        channel.subscribers += 1;
        Subscription {
            init: channel.init.clone(),
            config: channel.config.clone(),
            last: channel.last.clone(),
            meta: channel.meta.clone(),
            receiver: channel.sender.subscribe(),
        }
    }

    pub fn publisher(&self, key: StreamKey) -> broadcast::Sender<Arc<LivePacket>> {
        let capacity = self.capacity_for(key.mode);
        let mut guard = self.channels.lock().expect("live hub poisoned");
        let channel = guard.entry(key).or_insert_with(|| Channel::new(capacity));
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

    pub fn publish_packet(&self, key: &StreamKey, packet: LivePacket) {
        let guard = self.channels.lock().expect("live hub poisoned");
        let Some(channel) = guard.get(key) else {
            return;
        };
        channel.counters.packets.fetch_add(1, Ordering::Relaxed);
        channel
            .counters
            .bytes
            .fetch_add(packet.bytes.len() as u64, Ordering::Relaxed);
        if packet.keyframe && packet.kind != PacketKind::Config {
            channel.counters.keyframes.fetch_add(1, Ordering::Relaxed);
        }
        match packet.kind {
            PacketKind::Init => {
                if let Ok(mut slot) = channel.init.lock() {
                    *slot = Some(packet.bytes);
                }
                return;
            }
            PacketKind::Config => {
                if let Ok(mut slot) = channel.config.lock() {
                    *slot = Some(packet.bytes);
                }
                return;
            }
            PacketKind::Fragment | PacketKind::AccessUnit => {}
        }
        let shared = Arc::new(packet);
        if shared.keyframe {
            if let Ok(mut slot) = channel.last.lock() {
                *slot = Some(shared.clone());
            }
        }
        let _ = channel.sender.send(shared);
    }

    pub fn publish(&self, key: &StreamKey, fragment: Fragment) {
        self.publish_packet(key, LivePacket::fragment(fragment));
    }

    pub fn meta(&self, key: &StreamKey) -> Option<StreamMeta> {
        let guard = self.channels.lock().expect("live hub poisoned");
        guard
            .get(key)
            .and_then(|channel| channel.meta.lock().ok().map(|meta| meta.clone()))
    }

    pub fn set_meta(&self, key: &StreamKey, meta: StreamMeta) {
        let guard = self.channels.lock().expect("live hub poisoned");
        if let Some(channel) = guard.get(key) {
            if let Ok(mut slot) = channel.meta.lock() {
                *slot = meta;
            }
        }
    }

    pub fn note_audio(&self, key: &StreamKey) {
        let guard = self.channels.lock().expect("live hub poisoned");
        if let Some(channel) = guard.get(key) {
            channel
                .counters
                .audio_packets
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn note_drop(&self, key: &StreamKey) {
        let guard = self.channels.lock().expect("live hub poisoned");
        if let Some(channel) = guard.get(key) {
            channel.counters.drops.fetch_add(1, Ordering::Relaxed);
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

    pub fn snapshots(&self) -> Vec<StreamSnapshot> {
        let guard = self.channels.lock().expect("live hub poisoned");
        let mut out = Vec::with_capacity(guard.len());
        for (key, channel) in guard.iter() {
            let counters = &channel.counters;
            out.push(StreamSnapshot {
                device: key.device.clone(),
                channel: key.channel,
                sub_stream: key.sub_stream,
                mode: key.mode,
                subscribers: channel.sender.receiver_count(),
                publishers: channel.publishers,
                packets: counters.packets.load(Ordering::Relaxed),
                keyframes: counters.keyframes.load(Ordering::Relaxed),
                bytes: counters.bytes.load(Ordering::Relaxed),
                audio_packets: counters.audio_packets.load(Ordering::Relaxed),
                drops: counters.drops.load(Ordering::Relaxed),
                meta: channel
                    .meta
                    .lock()
                    .map(|meta| meta.clone())
                    .unwrap_or_default(),
            });
        }
        out
    }
}

pub fn spawn_publisher(hub: Arc<LiveHub>, key: StreamKey, init: Fragment, plan: Arc<ClipPlan>) {
    hub.publish_packet(&key, LivePacket::init(init));
    let sender = hub.publisher(key.clone());
    if plan.fragments.is_empty() {
        hub.publisher_done(&key);
        return;
    }
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

    fn key(mode: LiveMode) -> StreamKey {
        StreamKey::new("dev", 1, false, mode)
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
        let stream = key(LiveMode::Transcode);
        let subscription = hub.subscribe(stream.clone());
        assert_eq!(hub.active_streams(), 1);
        drop(subscription);
        hub.release(&stream);
        assert_eq!(hub.active_streams(), 0);
    }

    #[tokio::test]
    async fn a_late_subscriber_sees_the_stored_init_segment() {
        let hub = LiveHub::new(8);
        let stream = key(LiveMode::Transcode);
        let plan = Arc::new(plan_clip(&track(), &samples(25), 25));
        let first = hub.subscribe(stream.clone());
        hub.publish_packet(&stream, LivePacket::init(plan.init.clone()));
        assert_eq!(first.init.lock().unwrap().clone(), Some(plan.init.clone()));
        let late = hub.subscribe(stream.clone());
        assert_eq!(late.init.lock().unwrap().clone(), Some(plan.init.clone()));
        hub.release(&stream);
        hub.release(&stream);
    }

    #[tokio::test]
    async fn fragments_reach_every_subscriber() {
        let hub = Arc::new(LiveHub::new(8));
        let stream = key(LiveMode::Transcode);
        let first = hub.subscribe(stream.clone());
        let mut second = hub.subscribe(stream.clone()).receiver;
        let mut receiver = first.receiver;
        let plan = Arc::new(plan_clip(&track(), &samples(25), 25));
        spawn_publisher(hub.clone(), stream.clone(), plan.init.clone(), plan.clone());
        let one = receiver.recv().await.expect("first viewer fragment");
        let two = second.recv().await.expect("second viewer fragment");
        assert!(one.keyframe);
        assert_eq!(one, two);
        hub.release(&stream);
        hub.release(&stream);
    }

    #[tokio::test]
    async fn only_keyframe_packets_are_replayed_to_a_new_viewer() {
        let hub = LiveHub::new(8);
        let stream = key(LiveMode::Passthrough);
        let subscription = hub.subscribe(stream.clone());
        drop(subscription);
        let delta = LivePacket::access_unit(Fragment::from_static(b"delta"), false, 1);
        let idr = LivePacket::access_unit(Fragment::from_static(b"idr"), true, 2);
        hub.publish_packet(&stream, delta.clone());
        hub.publish_packet(&stream, idr.clone());
        let late = hub.subscribe(stream.clone());
        let stored = late.last.lock().unwrap().clone().expect("stored keyframe");
        assert_eq!(stored.as_ref(), &idr);
        hub.release(&stream);
        hub.release(&stream);
    }

    #[tokio::test]
    async fn a_lagging_viewer_sees_the_skip_count_and_the_next_packet() {
        let hub = Arc::new(LiveHub::new(2));
        let stream = key(LiveMode::Transcode);
        let mut receiver = hub.subscribe(stream.clone()).receiver;
        let sender = hub.publisher(stream.clone());
        for index in 0..5u8 {
            let _ = sender.send(Arc::new(LivePacket::fragment(Fragment::from(vec![index]))));
        }
        let error = receiver.recv().await.expect_err("the receiver lagged");
        assert!(matches!(
            error,
            broadcast::error::RecvError::Lagged(skipped) if skipped > 0
        ));
        let next = receiver.recv().await.expect("a packet after the lag");
        assert_eq!(next.bytes.as_ref(), &[3u8]);
        hub.release(&stream);
    }

    #[tokio::test]
    async fn audio_packets_and_drops_are_counted_per_stream() {
        let hub = LiveHub::new(8);
        let stream = key(LiveMode::Passthrough);
        let subscription = hub.subscribe(stream.clone());
        drop(subscription);
        hub.note_audio(&stream);
        hub.note_audio(&stream);
        hub.note_drop(&stream);
        hub.publish_packet(&stream, LivePacket::config(Fragment::from_static(b"hvcc")));
        hub.publish_packet(
            &stream,
            LivePacket::access_unit(Fragment::from_static(b"idr"), true, 0),
        );
        let snapshot = hub.snapshots().pop().expect("one stream");
        assert_eq!(snapshot.audio_packets, 2);
        assert_eq!(snapshot.drops, 1);
        assert_eq!(snapshot.keyframes, 1);
        assert_eq!(snapshot.mode, LiveMode::Passthrough);
        hub.release(&stream);
    }

    #[test]
    fn a_cloud_key_is_scoped_away_from_a_local_key_for_the_same_device() {
        let local = StreamKey::new("7f92970ddf5282743hrc", 1, false, LiveMode::Transcode);
        let cloud = StreamKey::cloud(
            "6f8c33b29c95774c14bae9436e721c8",
            "7f92970ddf5282743hrc",
            1,
            false,
            LiveMode::Transcode,
        );
        assert_ne!(local, cloud);
        let other = StreamKey::cloud(
            "11111111111111111111111111111111",
            "7f92970ddf5282743hrc",
            1,
            false,
            LiveMode::Transcode,
        );
        assert_ne!(cloud, other);
    }
}
