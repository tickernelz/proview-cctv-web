use crate::dvrip::playback::{codec_of, is_video, snap_rate, FrameRate, Record, RecordKind};
use crate::media::fmp4::{
    build_avcc, fragmented_init, ftyp, full_box, media_segment, movie, visual_sample_entry, Sample,
    MOVIE_TIMESCALE, TIMESCALE,
};
use crate::media::{h264, hevc, Codec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfig {
    pub codec: Codec,
    pub codec_string: String,
    pub width: u32,
    pub height: u32,
    pub reorders: bool,
    sample_entry: Vec<u8>,
}

impl VideoConfig {
    pub fn hevc(vps: &[u8], sps: &[u8], pps: &[u8]) -> Result<Self, String> {
        let parsed = hevc::parse_sps(sps)?;
        let hvcc = hevc::build_hvcc(&parsed, vps, sps, pps);
        Ok(Self {
            codec: Codec::H265,
            codec_string: hevc::codec_string(&parsed.profile),
            width: parsed.width,
            height: parsed.height,
            reorders: parsed.max_num_reorder.is_none_or(|frames| frames > 0),
            sample_entry: visual_sample_entry(b"hvc1", parsed.width, parsed.height, b"hvcC", &hvcc),
        })
    }

    pub fn h264(sps: &[u8], pps: &[u8]) -> Result<Self, String> {
        if h264::nal_type(pps) != Some(8) {
            return Err("the picture parameter set is missing".to_string());
        }
        let parsed = h264::parse_sps(sps).map_err(|error| format!("{error:?}"))?;
        let width = parsed.width.max(16);
        let height = parsed.height.max(16);
        Ok(Self {
            codec: Codec::H264,
            codec_string: format!(
                "avc1.{:02X}{:02X}{:02X}",
                sps.get(1).copied().unwrap_or(0x42),
                sps.get(2).copied().unwrap_or(0),
                sps.get(3).copied().unwrap_or(0x1e)
            ),
            width,
            height,
            reorders: parsed.may_reorder,
            sample_entry: visual_sample_entry(
                b"avc1",
                width,
                height,
                b"avcC",
                &build_avcc(sps, pps),
            ),
        })
    }

    pub fn from_nals(codec: Codec, nals: &[Vec<u8>]) -> Result<Self, String> {
        let find = |wanted: u8| {
            nals.iter()
                .find(|nal| nal_kind(codec, nal) == Some(wanted))
                .map(Vec::as_slice)
        };
        match codec {
            Codec::H265 => match (find(32), find(33), find(34)) {
                (Some(vps), Some(sps), Some(pps)) => Self::hevc(vps, sps, pps),
                _ => Err("the keyframe lacks VPS, SPS or PPS".to_string()),
            },
            Codec::H264 => match (find(7), find(8)) {
                (Some(sps), Some(pps)) => Self::h264(sps, pps),
                _ => Err("the keyframe lacks SPS or PPS".to_string()),
            },
        }
    }

    pub fn mime(&self) -> String {
        format!("video/mp4; codecs=\"{}\"", self.codec_string)
    }

    pub fn fragmented_init(&self) -> Vec<u8> {
        fragmented_init(self.width, self.height, &self.sample_entry)
    }
}

fn nal_kind(codec: Codec, nal: &[u8]) -> Option<u8> {
    let first = *nal.first()?;
    Some(match codec {
        Codec::H265 => (first >> 1) & 0x3f,
        Codec::H264 => first & 0x1f,
    })
}

fn out_of_band(codec: Codec, kind: u8) -> bool {
    match codec {
        Codec::H265 => matches!(kind, 32..=35),
        Codec::H264 => matches!(kind, 7..=9),
    }
}

pub fn is_keyframe<N: AsRef<[u8]>>(codec: Codec, nals: &[N]) -> bool {
    nals.iter()
        .any(|nal| match (codec, nal_kind(codec, nal.as_ref())) {
            (Codec::H265, Some(kind)) => (16..=21).contains(&kind),
            (Codec::H264, Some(kind)) => kind == 5,
            _ => false,
        })
}

pub fn sample_data<N: AsRef<[u8]>>(codec: Codec, nals: &[N]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nals.iter().map(|nal| nal.as_ref().len() + 4).sum());
    append_sample(codec, nals, &mut out);
    out
}

fn append_sample<N: AsRef<[u8]>>(codec: Codec, nals: &[N], out: &mut Vec<u8>) -> usize {
    let start = out.len();
    for nal in nals {
        let nal = nal.as_ref();
        let Some(kind) = nal_kind(codec, nal) else {
            continue;
        };
        if out_of_band(codec, kind) {
            continue;
        }
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out.len() - start
}

fn units(bytes: &[u8]) -> Vec<&[u8]> {
    crate::media::nal::split_annexb(bytes)
        .into_iter()
        .map(|unit| unit.data)
        .collect()
}

pub fn split_annexb(bytes: &[u8]) -> Vec<Vec<u8>> {
    crate::media::nal::split_annexb(bytes)
        .into_iter()
        .map(|unit| unit.data.to_vec())
        .collect()
}

#[derive(Debug, Default)]
pub struct Ticks {
    carry: f64,
}

impl Ticks {
    pub fn next(&mut self, per_second: f64) -> u32 {
        let rate = if per_second.is_finite() && per_second >= 1.0 {
            per_second
        } else {
            DEFAULT_RATE
        };
        self.carry += f64::from(TIMESCALE) / rate;
        let whole = self.carry.floor();
        self.carry -= whole;
        whole.max(1.0) as u32
    }
}

pub const DEFAULT_RATE: f64 = 12.0;

pub struct FragmentMux {
    sequence: u32,
    base: u64,
    samples: Vec<Sample>,
    per_fragment: usize,
}

impl FragmentMux {
    pub fn new(per_fragment: usize) -> Self {
        Self {
            sequence: 1,
            base: 0,
            samples: Vec::new(),
            per_fragment: per_fragment.max(1),
        }
    }

    pub fn push(&mut self, sample: Sample) -> Vec<(Vec<u8>, bool)> {
        let mut out = Vec::new();
        if sample.keyframe {
            out.extend(self.flush());
        }
        self.samples.push(sample);
        if self.samples.len() >= self.per_fragment {
            out.extend(self.flush());
        }
        out
    }

    pub fn flush(&mut self) -> Option<(Vec<u8>, bool)> {
        if self.samples.is_empty() {
            return None;
        }
        let samples = std::mem::take(&mut self.samples);
        let keyframe = samples[0].keyframe;
        let segment = media_segment(self.sequence, self.base, &samples);
        self.base += samples
            .iter()
            .map(|sample| u64::from(sample.duration))
            .sum::<u64>();
        self.sequence = self.sequence.wrapping_add(1);
        Some((segment, keyframe))
    }
}

pub struct LiveRate {
    header: Option<u8>,
    rate: FrameRate,
}

impl Default for LiveRate {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveRate {
    pub fn new() -> Self {
        Self {
            header: None,
            rate: FrameRate::default(),
        }
    }

    pub fn observe(&mut self, keyframe: bool, stamp: Option<u32>, header: Option<u8>) {
        if let Some(fps) = header.filter(|fps| (1..=60).contains(fps)) {
            self.header = Some(fps);
        }
        let record = Record {
            kind: if keyframe {
                RecordKind::Key
            } else {
                RecordKind::Delta
            },
            stamp: if keyframe { stamp } else { None },
            bytes: Vec::new(),
            rate: None,
        };
        self.rate.observe(&record);
    }

    pub fn per_second(&self) -> f64 {
        if self.rate.settled() {
            return self.rate.per_second;
        }
        self.header.map(f64::from).unwrap_or(DEFAULT_RATE)
    }

    pub fn settled(&self) -> bool {
        self.rate.settled()
    }
}

#[derive(Debug, Default)]
struct ClipRate {
    frames: usize,
    first: Option<(i64, usize)>,
    last: Option<(i64, usize)>,
    header: Option<u8>,
}

impl ClipRate {
    fn observe(&mut self, record: &Record) {
        if record.kind == RecordKind::Key {
            if self.header.is_none() {
                self.header = record.rate.filter(|fps| (1..=60).contains(fps));
            }
            if let Some(seconds) = record.stamp.and_then(crate::dvrip::playback::stamp_seconds) {
                if self.first.is_none() {
                    self.first = Some((seconds, self.frames));
                }
                self.last = Some((seconds, self.frames));
            }
        }
        self.frames += 1;
    }

    fn rate(&self) -> (f64, &'static str) {
        if let (Some((start, at_start)), Some((end, at_end))) = (self.first, self.last) {
            let span = end - start;
            if span >= 2 {
                let measured = (at_end - at_start) as f64 / span as f64;
                if (1.0..=60.0).contains(&measured) {
                    return (snap_rate(measured), "stamps");
                }
            }
        }
        match self.header {
            Some(fps) => (f64::from(fps), "header"),
            None => (DEFAULT_RATE, "default"),
        }
    }
}

pub fn clip_rate(records: &[Record]) -> (f64, &'static str) {
    let mut rate = ClipRate::default();
    for record in records.iter().filter(|record| is_video(record)) {
        rate.observe(record);
    }
    rate.rate()
}

enum ClipState {
    Waiting,
    Mux(VideoConfig),
    Raw(Codec),
}

pub struct ClipMux {
    state: ClipState,
    rate: ClipRate,
    bytes: Vec<u8>,
    sizes: Vec<u32>,
    keys: Vec<u32>,
}

#[derive(Debug)]
pub struct ClipFile {
    pub bytes: Vec<u8>,
    pub codec: Codec,
    pub muxed: bool,
    pub frames: usize,
    pub fps: f64,
    pub fps_source: &'static str,
    pub seconds: f64,
}

const MDAT_HEADER: usize = 16;

impl ClipMux {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: ClipState::Waiting,
            rate: ClipRate::default(),
            bytes: Vec::with_capacity(capacity),
            sizes: Vec::new(),
            keys: Vec::new(),
        }
    }

    pub fn held(&self) -> usize {
        self.bytes.len()
    }

    pub fn push(&mut self, record: Record) {
        if !is_video(&record) {
            return;
        }
        self.rate.observe(&record);
        if matches!(self.state, ClipState::Waiting) {
            if record.kind != RecordKind::Key {
                return;
            }
            let owned = split_annexb(&record.bytes);
            let Some(codec) = codec_of(&owned) else {
                return;
            };
            let Ok(config) = VideoConfig::from_nals(codec, &owned) else {
                return;
            };
            if config.reorders {
                self.state = ClipState::Raw(codec);
            } else {
                self.bytes.extend(ftyp(brands(codec)));
                self.bytes.extend_from_slice(&[0, 0, 0, 1]);
                self.bytes.extend_from_slice(b"mdat");
                self.bytes.extend_from_slice(&[0u8; 8]);
                self.state = ClipState::Mux(config);
            }
        }
        match &self.state {
            ClipState::Waiting => {}
            ClipState::Raw(_) => self.bytes.extend_from_slice(&record.bytes),
            ClipState::Mux(config) => {
                let codec = config.codec;
                let nals = units(&record.bytes);
                let keyframe = is_keyframe(codec, &nals);
                if self.sizes.is_empty() && !keyframe {
                    return;
                }
                let size = append_sample(codec, &nals, &mut self.bytes);
                if size == 0 {
                    return;
                }
                self.sizes.push(size as u32);
                if keyframe {
                    self.keys.push(self.sizes.len() as u32);
                }
            }
        }
    }

    pub fn finish(self) -> Result<ClipFile, String> {
        let (fps, fps_source) = self.rate.rate();
        let ClipMux {
            state,
            mut bytes,
            sizes,
            keys,
            ..
        } = self;
        match state {
            ClipState::Waiting => Err(
                "the clip carries no keyframe with parameter sets, so its codec is unknown"
                    .to_string(),
            ),
            ClipState::Raw(codec) => Ok(ClipFile {
                bytes,
                codec,
                muxed: false,
                frames: 0,
                fps,
                fps_source,
                seconds: 0.0,
            }),
            ClipState::Mux(config) => {
                if sizes.is_empty() {
                    return Err("the clip holds no keyframe".to_string());
                }
                let head = ftyp(brands(config.codec)).len();
                let payload = (bytes.len() - head) as u64;
                bytes[head + 8..head + MDAT_HEADER].copy_from_slice(&payload.to_be_bytes());
                let mut ticks = Ticks::default();
                let durations: Vec<u32> = sizes.iter().map(|_| ticks.next(fps)).collect();
                let moov = progressive_moov(
                    &config,
                    &sizes,
                    &durations,
                    &keys,
                    (head + MDAT_HEADER) as u64,
                );
                bytes.extend(moov);
                let frames = sizes.len();
                Ok(ClipFile {
                    bytes,
                    codec: config.codec,
                    muxed: true,
                    frames,
                    fps,
                    fps_source,
                    seconds: frames as f64 / fps,
                })
            }
        }
    }
}

fn brands(codec: Codec) -> &'static [u8] {
    match codec {
        Codec::H265 => b"isomiso2hvc1mp41",
        Codec::H264 => b"isomiso2avc1mp41",
    }
}

pub fn progressive_moov(
    config: &VideoConfig,
    sizes: &[u32],
    durations: &[u32],
    keys: &[u32],
    offset: u64,
) -> Vec<u8> {
    let media_duration: u64 = durations.iter().map(|duration| u64::from(*duration)).sum();
    let movie_duration = media_duration * u64::from(MOVIE_TIMESCALE) / u64::from(TIMESCALE);

    let mut stts_runs: Vec<(u32, u32)> = Vec::new();
    for duration in durations {
        match stts_runs.last_mut() {
            Some((count, delta)) if delta == duration => *count += 1,
            _ => stts_runs.push((1, *duration)),
        }
    }
    let mut stts = Vec::with_capacity(4 + stts_runs.len() * 8);
    stts.extend_from_slice(&(stts_runs.len() as u32).to_be_bytes());
    for (count, delta) in &stts_runs {
        stts.extend_from_slice(&count.to_be_bytes());
        stts.extend_from_slice(&delta.to_be_bytes());
    }

    let mut stss = Vec::with_capacity(4 + keys.len() * 4);
    stss.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    for key in keys {
        stss.extend_from_slice(&key.to_be_bytes());
    }

    let chunks: u32 = u32::from(!sizes.is_empty());
    let mut stsc = Vec::new();
    stsc.extend_from_slice(&chunks.to_be_bytes());
    if chunks == 1 {
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
    }

    let mut stsz = Vec::with_capacity(8 + sizes.len() * 4);
    stsz.extend_from_slice(&0u32.to_be_bytes());
    stsz.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
    for size in sizes {
        stsz.extend_from_slice(&size.to_be_bytes());
    }

    let mut co64 = Vec::new();
    co64.extend_from_slice(&chunks.to_be_bytes());
    if chunks == 1 {
        co64.extend_from_slice(&offset.to_be_bytes());
    }

    let mut stsd = Vec::new();
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend_from_slice(&config.sample_entry);
    let mut stbl = full_box(b"stsd", 0, 0, &stsd);
    stbl.extend(full_box(b"stts", 0, 0, &stts));
    if keys.len() < sizes.len() {
        stbl.extend(full_box(b"stss", 0, 0, &stss));
    }
    stbl.extend(full_box(b"stsc", 0, 0, &stsc));
    stbl.extend(full_box(b"stsz", 0, 0, &stsz));
    stbl.extend(full_box(b"co64", 0, 0, &co64));
    movie(
        config.width,
        config.height,
        u32::try_from(movie_duration).unwrap_or(u32::MAX),
        u32::try_from(media_duration).unwrap_or(u32::MAX),
        &stbl,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvrip::playback::MediaReader;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    fn vps() -> Vec<u8> {
        hex("40010c01ffff0160000003000003000003000003003f2c0c00000fa00000bb8140")
    }

    fn sps() -> Vec<u8> {
        hex("4201010160000003000003000003000003003fa001202002887f9cbabc12e080")
    }

    fn pps() -> Vec<u8> {
        hex("4401c0f3c0e14524")
    }

    fn top_boxes(data: &[u8]) -> Vec<(String, usize, usize)> {
        let mut out = Vec::new();
        let mut index = 0;
        while index + 8 <= data.len() {
            let size = u32::from_be_bytes([
                data[index],
                data[index + 1],
                data[index + 2],
                data[index + 3],
            ]) as usize;
            let size = if size == 1 {
                u64::from_be_bytes(data[index + 8..index + 16].try_into().expect("u64")) as usize
            } else {
                size
            };
            if size < 8 {
                break;
            }
            out.push((
                String::from_utf8_lossy(&data[index + 4..index + 8]).to_string(),
                index,
                size,
            ));
            index += size;
        }
        out
    }

    fn find(data: &[u8], kind: &[u8; 4]) -> usize {
        data.windows(4)
            .position(|window| window == kind)
            .expect("box present")
    }

    fn word(data: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
    }

    #[test]
    fn the_dual_lens_parameter_sets_describe_a_2304_by_2592_hvc1_track() {
        let config = VideoConfig::hevc(&vps(), &sps(), &pps()).expect("config");
        assert_eq!(config.codec, Codec::H265);
        assert_eq!((config.width, config.height), (2304, 2592));
        assert_eq!(config.codec_string, "hvc1.1.6.L63");
        assert_eq!(config.mime(), "video/mp4; codecs=\"hvc1.1.6.L63\"");
        let init = config.fragmented_init();
        let at = find(&init, b"hvc1");
        assert_eq!(u16::from_be_bytes([init[at + 28], init[at + 29]]), 2304);
        assert_eq!(u16::from_be_bytes([init[at + 30], init[at + 31]]), 2592);
        let hvcc = find(&init, b"hvcC");
        assert_eq!(init[hvcc + 4], 1);
        assert!(init
            .windows(sps().len())
            .any(|window| window == sps().as_slice()));
        assert!(!init.windows(4).any(|window| window == b"avcC"));
        assert!(init.windows(4).any(|window| window == b"mvex"));
    }

    #[test]
    fn the_config_is_found_among_the_keyframe_nals_for_either_codec() {
        let units = vec![vps(), sps(), pps(), vec![0x26, 0x01, 0xaf]];
        let config = VideoConfig::from_nals(Codec::H265, &units).expect("hevc");
        assert_eq!(config.width, 2304);
        assert!(VideoConfig::from_nals(Codec::H265, &units[1..]).is_err());
        let avc = vec![
            vec![0x67, 0x42, 0xc0, 0x1e, 0xda, 0x02, 0x80, 0xbf, 0xe5, 0x84],
            vec![0x68, 0xce, 0x3c, 0x80],
            vec![0x65, 0x88],
        ];
        let config = VideoConfig::from_nals(Codec::H264, &avc).expect("avc");
        assert_eq!(config.codec_string, "avc1.42C01E");
        assert!(config
            .fragmented_init()
            .windows(4)
            .any(|window| window == b"avcC"));
    }

    #[test]
    fn samples_carry_only_the_picture_nals_with_length_prefixes() {
        let nals = vec![
            vps(),
            sps(),
            pps(),
            vec![0x4e, 0x01, 0x05],
            vec![0x26, 0x01, 0xaf],
        ];
        let data = sample_data(Codec::H265, &nals);
        assert_eq!(
            data,
            [
                &[0, 0, 0, 3, 0x4e, 0x01, 0x05][..],
                &[0, 0, 0, 3, 0x26, 0x01, 0xaf]
            ]
            .concat()
        );
        assert!(is_keyframe(Codec::H265, &nals));
        assert!(!is_keyframe(Codec::H265, &[vec![0x02, 0x01, 0xd0]]));
        let avc = vec![
            vec![0x09, 0xf0],
            vec![0x67, 0x42],
            vec![0x68, 0xce],
            vec![0x65, 0x88],
        ];
        assert_eq!(sample_data(Codec::H264, &avc), vec![0, 0, 0, 2, 0x65, 0x88]);
        assert!(is_keyframe(Codec::H264, &avc));
        let split = split_annexb(&[0, 0, 0, 1, 0x26, 0x01, 0, 0, 1, 0x02, 0x01]);
        assert_eq!(split, vec![vec![0x26, 0x01], vec![0x02, 0x01]]);
    }

    #[test]
    fn a_live_mux_emits_one_fragment_per_frame_with_the_keyframe_flag() {
        let mut mux = FragmentMux::new(1);
        let sample = |keyframe: bool| Sample {
            data: vec![0, 0, 0, 1, 0x26],
            duration: 7200,
            keyframe,
        };
        let first = mux.push(sample(true));
        assert_eq!(first.len(), 1);
        assert!(first[0].1);
        let second = mux.push(sample(false));
        assert_eq!(second.len(), 1);
        assert!(!second[0].1);
        let key_trun = find(&first[0].0, b"trun");
        assert_eq!(word(&first[0].0, key_trun + 4), 0x0705);
        assert_eq!(word(&first[0].0, key_trun + 16), 0x0200_0000);
        assert_eq!(word(&first[0].0, key_trun + 28), 0x0200_0000);
        let delta_trun = find(&second[0].0, b"trun");
        assert_eq!(
            word(&second[0].0, delta_trun + 16),
            0x0101_0000,
            "a delta must not open its fragment as a sync sample"
        );
        assert_eq!(word(&second[0].0, delta_trun + 28), 0x0101_0000);
        let tfdt = find(&second[0].0, b"tfdt");
        let base = u64::from_be_bytes(second[0].0[tfdt + 8..tfdt + 16].try_into().expect("u64"));
        assert_eq!(base, 7200);
        assert!(mux.flush().is_none());
    }

    #[test]
    fn a_clip_mux_cuts_a_fragment_at_every_keyframe() {
        let mut mux = FragmentMux::new(12);
        let sample = |keyframe: bool| Sample {
            data: vec![1],
            duration: 3000,
            keyframe,
        };
        assert!(mux.push(sample(true)).is_empty());
        assert!(mux.push(sample(false)).is_empty());
        let cut = mux.push(sample(true));
        assert_eq!(cut.len(), 1);
        assert!(cut[0].1);
        assert_eq!(word(&cut[0].0, find(&cut[0].0, b"trun") + 8), 2);
        let tail = mux.flush().expect("tail");
        assert!(tail.1);
    }

    #[test]
    fn ticks_keep_a_fractional_rate_exact_over_a_second() {
        let mut ticks = Ticks::default();
        let total: u32 = (0..25).map(|_| ticks.next(12.5)).sum();
        assert_eq!(total, 2 * TIMESCALE);
        let mut ticks = Ticks::default();
        assert_eq!(
            ticks.next(0.0),
            (f64::from(TIMESCALE) / DEFAULT_RATE) as u32
        );
        assert_eq!(
            ticks.next(f64::NAN),
            (f64::from(TIMESCALE) / DEFAULT_RATE) as u32
        );
    }

    fn packed(hour: u32, minute: u32, second: u32) -> u32 {
        (26u32 << 26) | (10 << 22) | (24 << 17) | (hour << 12) | (minute << 6) | second
    }

    fn key(stamp: u32, rate: u8) -> Record {
        Record {
            kind: RecordKind::Key,
            stamp: Some(stamp),
            bytes: Vec::new(),
            rate: Some(rate),
        }
    }

    fn delta() -> Record {
        Record {
            kind: RecordKind::Delta,
            stamp: None,
            bytes: Vec::new(),
            rate: None,
        }
    }

    fn gops(keys: usize, per_gop: usize, seconds_per_gop: &[u32]) -> Vec<Record> {
        let mut records = Vec::new();
        let mut second = 0u32;
        for index in 0..keys {
            records.push(key(packed(10, 34 + second / 60, second % 60), 12));
            for _ in 1..per_gop {
                records.push(delta());
            }
            second += seconds_per_gop[index % seconds_per_gop.len()];
        }
        records
    }

    #[test]
    fn the_clip_rate_comes_from_the_record_stamps_not_the_header_byte() {
        let records = gops(40, 36, &[3, 3, 3, 2, 3, 3, 3, 3]);
        let (rate, source) = clip_rate(&records);
        assert_eq!(source, "stamps");
        assert_eq!(rate, 12.5);
    }

    #[test]
    fn a_clip_without_a_stamp_span_falls_back_to_the_header_then_the_default() {
        let records = gops(1, 10, &[3]);
        assert_eq!(clip_rate(&records), (12.0, "header"));
        let mut bare = records.clone();
        bare[0].rate = None;
        assert_eq!(clip_rate(&bare), (DEFAULT_RATE, "default"));
    }

    #[test]
    fn a_live_rate_uses_the_header_until_the_stamps_settle() {
        let mut rate = LiveRate::new();
        assert_eq!(rate.per_second(), DEFAULT_RATE);
        for gop in 0..5u32 {
            rate.observe(true, Some(packed(10, 0, gop * 2)), Some(12));
            assert_eq!(rate.per_second(), 12.0);
            for _ in 1..25 {
                rate.observe(false, None, None);
            }
        }
        assert!(!rate.settled());
        rate.observe(true, Some(packed(10, 0, 10)), Some(12));
        assert!(rate.settled());
        assert_eq!(rate.per_second(), 12.5);
    }

    #[test]
    fn a_progressive_file_indexes_every_sample_and_its_duration() {
        let records = clip(75, 25, &sps());
        let mut mux = ClipMux::new(1 << 16);
        for record in records {
            mux.push(record);
        }
        let clip = mux.finish().expect("clip");
        assert!(clip.muxed);
        assert_eq!(clip.frames, 75);
        assert_eq!((clip.fps, clip.fps_source), (12.5, "stamps"));
        let file = clip.bytes;
        let top = top_boxes(&file);
        let kinds: Vec<&str> = top.iter().map(|(kind, _, _)| kind.as_str()).collect();
        assert_eq!(kinds, ["ftyp", "mdat", "moov"]);
        assert_eq!(top[2].1 + top[2].2, file.len());
        let mdhd = find(&file, b"mdhd");
        assert_eq!(word(&file, mdhd + 16), TIMESCALE);
        assert_eq!(word(&file, mdhd + 20), 6 * TIMESCALE);
        let mvhd = find(&file, b"mvhd");
        assert_eq!(word(&file, mvhd + 16), MOVIE_TIMESCALE);
        assert_eq!(word(&file, mvhd + 20), 6000);
        let stss = find(&file, b"stss");
        assert_eq!(word(&file, stss + 8), 3);
        assert_eq!(
            [
                word(&file, stss + 12),
                word(&file, stss + 16),
                word(&file, stss + 20)
            ],
            [1, 26, 51]
        );
        let stsz = find(&file, b"stsz");
        assert_eq!(word(&file, stsz + 12), 75);
        let co64 = find(&file, b"co64");
        let offset =
            u64::from_be_bytes(file[co64 + 12..co64 + 20].try_into().expect("u64")) as usize;
        assert_eq!(offset, top[1].1 + 16);
        assert_eq!(&file[offset..offset + 7], &[0, 0, 0, 3, 0x26, 0x01, 0x80]);
        assert_eq!(
            &file[offset + 7..offset + 14],
            &[0, 0, 0, 3, 0x02, 0x01, 0x81]
        );
        let stts = find(&file, b"stts");
        let runs = word(&file, stts + 8) as usize;
        let total: u64 = (0..runs)
            .map(|run| {
                u64::from(word(&file, stts + 12 + run * 8))
                    * u64::from(word(&file, stts + 16 + run * 8))
            })
            .sum();
        assert_eq!(total, 6 * u64::from(TIMESCALE));
    }

    fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }

    fn clip(frames: usize, gop: usize, sps: &[u8]) -> Vec<Record> {
        (0..frames)
            .map(|index| {
                if index % gop == 0 {
                    let second = (index as f64 / 12.5) as u32;
                    Record {
                        kind: RecordKind::Key,
                        stamp: Some(packed(10, 0, second)),
                        bytes: annexb(&[
                            vps(),
                            sps.to_vec(),
                            pps(),
                            vec![0x26, 0x01, 0x80 | index as u8],
                        ]),
                        rate: Some(12),
                    }
                } else {
                    Record {
                        kind: RecordKind::Delta,
                        stamp: None,
                        bytes: annexb(&[vec![0x02, 0x01, 0x80 | index as u8]]),
                        rate: None,
                    }
                }
            })
            .collect()
    }

    #[test]
    fn a_reordering_stream_is_kept_as_its_own_elementary_stream() {
        let b_frames = hex(
            "42010101600000030090000003000003003ca00a080f165959a4932bc05a020000030002000003003210",
        );
        let records = clip(30, 25, &b_frames);
        let raw: Vec<u8> = records
            .iter()
            .flat_map(|record| record.bytes.clone())
            .collect();
        let mut mux = ClipMux::new(0);
        mux.push(Record {
            kind: RecordKind::Pad,
            stamp: None,
            bytes: vec![0xd5; 320],
            rate: None,
        });
        for record in records {
            mux.push(record);
        }
        let clip = mux.finish().expect("raw clip");
        assert!(!clip.muxed);
        assert_eq!(clip.codec, Codec::H265);
        assert_eq!(clip.bytes, raw);
    }

    #[test]
    fn a_clip_without_a_keyframe_is_refused_with_its_reason() {
        let mut mux = ClipMux::new(0);
        mux.push(Record {
            kind: RecordKind::Delta,
            stamp: None,
            bytes: annexb(&[vec![0x02, 0x01, 0]]),
            rate: None,
        });
        let error = mux.finish().expect_err("no keyframe");
        assert!(error.contains("codec is unknown"), "{error}");
    }

    #[test]
    fn b_frame_encoders_are_told_apart_from_the_camera_by_their_sps() {
        let camera = VideoConfig::hevc(&vps(), &sps(), &pps()).expect("camera");
        assert!(
            !camera.reorders,
            "the dual-lens camera declares no reordering"
        );
        let x265_b4 = VideoConfig::hevc(
            &hex("40010c01ffff01600000030090000003000003003c959809"),
            &hex("42010101600000030090000003000003003ca00a080f165959a4932bc05a020000030002000003003210"),
            &hex("4401c172b46240"),
        )
        .expect("x265 bframes=4");
        assert!(x265_b4.reorders);
        let x265_b0 = VideoConfig::hevc(
            &hex("40010c01ffff01600000030090000003000003003c928090"),
            &hex("42010101600000030090000003000003003ca00a080f16592a4932bc05a02000000300200000030321"),
            &hex("4401c172b46240"),
        )
        .expect("x265 bframes=0");
        assert!(!x265_b0.reorders);
        let x264_b3 = VideoConfig::h264(
            &hex("6764000dacd94141fb011000000300100000030320f1429960"),
            &hex("68ebe3cb22c0"),
        )
        .expect("x264 high bf=3");
        assert!(x264_b3.reorders);
        let x264_b0 = VideoConfig::h264(
            &hex("674d400dd90141fb011000000300100000030320f142a480"),
            &hex("68ebc3cb20"),
        )
        .expect("x264 main bf=0");
        assert!(!x264_b0.reorders);
    }

    #[test]
    fn a_key_record_carries_its_encoder_frame_rate() {
        let mut reader = MediaReader::new();
        let mut bytes = hex("000001fc530c20440766b36a");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[0x26, 0x01, 0xaf]);
        bytes.extend_from_slice(&hex("000001fd"));
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&[0x02, 0x01]);
        reader.push(&bytes);
        let key = reader.next_record().expect("key");
        assert_eq!(key.rate, Some(12));
        let delta = reader.next_record().expect("delta");
        assert_eq!(delta.rate, None);
    }
}
