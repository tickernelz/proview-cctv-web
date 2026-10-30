pub const TIMESCALE: u32 = 90_000;
pub const MOVIE_TIMESCALE: u32 = 1000;

pub struct TrackConfig {
    pub width: u32,
    pub height: u32,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

pub struct Sample {
    pub data: Vec<u8>,
    pub duration: u32,
    pub keyframe: bool,
}

pub(crate) fn box_bytes(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

pub(crate) fn full_box(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + payload.len());
    body.push(version);
    body.extend_from_slice(&[(flags >> 16) as u8, (flags >> 8) as u8, flags as u8]);
    body.extend_from_slice(payload);
    box_bytes(kind, &body)
}

pub fn build_avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(11 + sps.len() + pps.len());
    out.push(1);
    out.push(sps.get(1).copied().unwrap_or(0x42));
    out.push(sps.get(2).copied().unwrap_or(0));
    out.push(sps.get(3).copied().unwrap_or(0x1e));
    out.push(0xff);
    out.push(0xe1);
    out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1);
    out.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    out.extend_from_slice(pps);
    out
}

pub fn init_segment(track: &TrackConfig) -> Vec<u8> {
    let entry = visual_sample_entry(
        b"avc1",
        track.width,
        track.height,
        b"avcC",
        &build_avcc(&track.sps, &track.pps),
    );
    fragmented_init(track.width, track.height, &entry)
}

pub fn visual_sample_entry(
    kind: &[u8; 4],
    width: u32,
    height: u32,
    config_kind: &[u8; 4],
    config: &[u8],
) -> Vec<u8> {
    let mut entry = Vec::new();
    entry.extend_from_slice(&[0u8; 6]);
    entry.extend_from_slice(&1u16.to_be_bytes());
    entry.extend_from_slice(&[0u8; 16]);
    entry.extend_from_slice(&(width as u16).to_be_bytes());
    entry.extend_from_slice(&(height as u16).to_be_bytes());
    entry.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    entry.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    entry.extend_from_slice(&0u32.to_be_bytes());
    entry.extend_from_slice(&1u16.to_be_bytes());
    entry.extend_from_slice(&[0u8; 32]);
    entry.extend_from_slice(&0x0018u16.to_be_bytes());
    entry.extend_from_slice(&0xffffu16.to_be_bytes());
    entry.extend(box_bytes(config_kind, config));
    box_bytes(kind, &entry)
}

pub fn fragmented_init(width: u32, height: u32, sample_entry: &[u8]) -> Vec<u8> {
    let mut stsd = Vec::new();
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend_from_slice(sample_entry);
    let mut stbl = full_box(b"stsd", 0, 0, &stsd);
    stbl.extend(full_box(b"stts", 0, 0, &0u32.to_be_bytes()));
    stbl.extend(full_box(b"stsc", 0, 0, &0u32.to_be_bytes()));
    stbl.extend(full_box(b"stsz", 0, 0, &[0u8; 8]));
    stbl.extend(full_box(b"stco", 0, 0, &0u32.to_be_bytes()));

    let mut trex = Vec::new();
    trex.extend_from_slice(&1u32.to_be_bytes());
    trex.extend_from_slice(&1u32.to_be_bytes());
    trex.extend_from_slice(&0u32.to_be_bytes());
    trex.extend_from_slice(&0u32.to_be_bytes());
    trex.extend_from_slice(&0u32.to_be_bytes());
    let mvex = box_bytes(b"mvex", &full_box(b"trex", 0, 0, &trex));

    let mut out = ftyp(b"isomiso5avc1mp41");
    out.extend(movie(width, height, 0, 0, &stbl, Some(&mvex)));
    out
}

pub(crate) fn ftyp(brands: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"isom");
    body.extend_from_slice(&512u32.to_be_bytes());
    body.extend_from_slice(brands);
    box_bytes(b"ftyp", &body)
}

pub(crate) fn movie(
    width: u32,
    height: u32,
    movie_duration: u32,
    media_duration: u32,
    stbl: &[u8],
    mvex: Option<&[u8]>,
) -> Vec<u8> {
    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&0u32.to_be_bytes());
    mvhd.extend_from_slice(&0u32.to_be_bytes());
    mvhd.extend_from_slice(&MOVIE_TIMESCALE.to_be_bytes());
    mvhd.extend_from_slice(&movie_duration.to_be_bytes());
    mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
    mvhd.extend_from_slice(&[0u8; 10]);
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        mvhd.extend_from_slice(&value.to_be_bytes());
    }
    for _ in 0..6 {
        mvhd.extend_from_slice(&0u32.to_be_bytes());
    }
    mvhd.extend_from_slice(&2u32.to_be_bytes());
    let mvhd = full_box(b"mvhd", 0, 0, &mvhd);

    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&0u32.to_be_bytes());
    tkhd.extend_from_slice(&0u32.to_be_bytes());
    tkhd.extend_from_slice(&1u32.to_be_bytes());
    tkhd.extend_from_slice(&0u32.to_be_bytes());
    tkhd.extend_from_slice(&movie_duration.to_be_bytes());
    tkhd.extend_from_slice(&[0u8; 8]);
    tkhd.extend_from_slice(&0u16.to_be_bytes());
    tkhd.extend_from_slice(&0u16.to_be_bytes());
    tkhd.extend_from_slice(&0u16.to_be_bytes());
    tkhd.extend_from_slice(&0u16.to_be_bytes());
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        tkhd.extend_from_slice(&value.to_be_bytes());
    }
    tkhd.extend_from_slice(&(width << 16).to_be_bytes());
    tkhd.extend_from_slice(&(height << 16).to_be_bytes());
    let tkhd = full_box(b"tkhd", 0, 3, &tkhd);

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&0u32.to_be_bytes());
    mdhd.extend_from_slice(&0u32.to_be_bytes());
    mdhd.extend_from_slice(&TIMESCALE.to_be_bytes());
    mdhd.extend_from_slice(&media_duration.to_be_bytes());
    mdhd.extend_from_slice(&0x55c4u16.to_be_bytes());
    mdhd.extend_from_slice(&0u16.to_be_bytes());
    let mdhd = full_box(b"mdhd", 0, 0, &mdhd);

    let mut hdlr = Vec::new();
    hdlr.extend_from_slice(&0u32.to_be_bytes());
    hdlr.extend_from_slice(b"vide");
    hdlr.extend_from_slice(&[0u8; 12]);
    hdlr.extend_from_slice(b"VideoHandler\0");
    let hdlr = full_box(b"hdlr", 0, 0, &hdlr);

    let mut vmhd = Vec::new();
    vmhd.extend_from_slice(&0u16.to_be_bytes());
    vmhd.extend_from_slice(&[0u8; 6]);
    let vmhd = full_box(b"vmhd", 0, 1, &vmhd);

    let mut dref = Vec::new();
    dref.extend_from_slice(&1u32.to_be_bytes());
    dref.extend(full_box(b"url ", 0, 1, &[]));
    let dinf = box_bytes(b"dinf", &full_box(b"dref", 0, 0, &dref));

    let mut minf = vmhd;
    minf.extend(dinf);
    minf.extend(box_bytes(b"stbl", stbl));
    let minf = box_bytes(b"minf", &minf);

    let mut mdia = mdhd;
    mdia.extend(hdlr);
    mdia.extend(minf);
    let mdia = box_bytes(b"mdia", &mdia);

    let mut trak = tkhd;
    trak.extend(mdia);
    let trak = box_bytes(b"trak", &trak);

    let mut moov = mvhd;
    moov.extend(trak);
    if let Some(mvex) = mvex {
        moov.extend_from_slice(mvex);
    }
    box_bytes(b"moov", &moov)
}

const SYNC_SAMPLE: u32 = 0x0200_0000;
const DEPENDENT_SAMPLE: u32 = 0x0101_0000;

fn sample_flags(sample: &Sample) -> u32 {
    if sample.keyframe {
        SYNC_SAMPLE
    } else {
        DEPENDENT_SAMPLE
    }
}

pub fn media_segment(sequence: u32, base_decode_time: u64, samples: &[Sample]) -> Vec<u8> {
    let mfhd = full_box(b"mfhd", 0, 0, &sequence.to_be_bytes());

    let mut tfhd = Vec::new();
    tfhd.extend_from_slice(&1u32.to_be_bytes());
    let tfhd = full_box(b"tfhd", 0, 0x020000, &tfhd);

    let tfdt_body: Vec<u8> = base_decode_time.to_be_bytes().to_vec();
    let tfdt = full_box(b"tfdt", 1, 0, &tfdt_body);

    let build_trun = |data_offset: u32| -> Vec<u8> {
        let mut trun = Vec::new();
        trun.extend_from_slice(&(samples.len() as u32).to_be_bytes());
        trun.extend_from_slice(&data_offset.to_be_bytes());
        let first = samples.first().map(sample_flags).unwrap_or(SYNC_SAMPLE);
        trun.extend_from_slice(&first.to_be_bytes());
        for sample in samples {
            trun.extend_from_slice(&sample.duration.to_be_bytes());
            trun.extend_from_slice(&(sample.data.len() as u32).to_be_bytes());
            trun.extend_from_slice(&sample_flags(sample).to_be_bytes());
        }
        full_box(b"trun", 0, 0x000705, &trun)
    };

    let assemble = |trun: Vec<u8>| -> Vec<u8> {
        let mut traf = Vec::new();
        traf.extend(tfhd.clone());
        traf.extend(tfdt.clone());
        traf.extend(trun);
        let traf = box_bytes(b"traf", &traf);
        let mut moof = Vec::new();
        moof.extend(mfhd.clone());
        moof.extend(traf);
        box_bytes(b"moof", &moof)
    };

    let probe = assemble(build_trun(0));
    let data_offset = (probe.len() + 8) as u32;
    let moof = assemble(build_trun(data_offset));

    let mut mdat_payload = Vec::new();
    for sample in samples {
        mdat_payload.extend_from_slice(&sample.data);
    }
    let mdat = box_bytes(b"mdat", &mdat_payload);

    let mut out = Vec::with_capacity(moof.len() + mdat.len());
    out.extend(moof);
    out.extend(mdat);
    out
}

pub fn avcc_sample(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + nal.len());
    out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
    out.extend_from_slice(nal);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(data: &[u8]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let mut index = 0;
        while index + 8 <= data.len() {
            let size = u32::from_be_bytes([
                data[index],
                data[index + 1],
                data[index + 2],
                data[index + 3],
            ]) as usize;
            let kind = String::from_utf8_lossy(&data[index + 4..index + 8]).to_string();
            if size < 8 {
                break;
            }
            out.push((kind, size));
            index += size;
        }
        out
    }

    fn track() -> TrackConfig {
        TrackConfig {
            width: 1280,
            height: 720,
            sps: vec![0x67, 66, 0, 30],
            pps: vec![0x68, 0xce, 0x3c, 0x80],
        }
    }

    #[test]
    fn avcc_carries_profile_level_and_both_parameter_sets() {
        let avcc = build_avcc(&[0x67, 0x42, 0xc0, 0x1e], &[0x68, 0xce]);
        assert_eq!(avcc[0], 1);
        assert_eq!(avcc[1], 0x42);
        assert_eq!(avcc[2], 0xc0);
        assert_eq!(avcc[3], 0x1e);
        assert_eq!(avcc[4], 0xff);
        assert_eq!(avcc[5], 0xe1);
        assert_eq!(u16::from_be_bytes([avcc[6], avcc[7]]), 4);
        assert_eq!(&avcc[8..12], &[0x67, 0x42, 0xc0, 0x1e]);
        assert_eq!(avcc[12], 1);
        assert_eq!(u16::from_be_bytes([avcc[13], avcc[14]]), 2);
        assert_eq!(&avcc[15..17], &[0x68, 0xce]);
    }

    #[test]
    fn init_segment_is_ftyp_then_moov_with_mvex() {
        let init = init_segment(&track());
        let top = boxes(&init);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "ftyp");
        assert_eq!(top[1].0, "moov");
        assert_eq!(top[0].1 + top[1].1, init.len());

        let moov = &init[top[0].1..];
        let children: Vec<String> = boxes(&moov[8..])
            .into_iter()
            .map(|(kind, _)| kind)
            .collect();
        assert_eq!(children, vec!["mvhd", "trak", "mvex"]);
    }

    #[test]
    fn media_segment_pairs_moof_and_mdat_with_matching_payload() {
        let samples = vec![
            Sample {
                data: avcc_sample(&[0x65, 0x88]),
                duration: 3000,
                keyframe: true,
            },
            Sample {
                data: avcc_sample(&[0x41, 0x9a]),
                duration: 3000,
                keyframe: false,
            },
        ];
        let segment = media_segment(1, 0, &samples);
        let top = boxes(&segment);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "moof");
        assert_eq!(top[1].0, "mdat");
        let mdat_payload = &segment[top[0].1 + 8..];
        assert_eq!(mdat_payload.len(), 6 + 6);
        assert_eq!(&mdat_payload[..4], &2u32.to_be_bytes());
        assert_eq!(&mdat_payload[6..10], &2u32.to_be_bytes());
    }

    #[test]
    fn trun_records_sample_count_and_per_sample_sizes() {
        let samples = vec![Sample {
            data: avcc_sample(&[0x65, 0x11, 0x22]),
            duration: 1500,
            keyframe: true,
        }];
        let segment = media_segment(7, 90_000, &samples);
        let moof_size = boxes(&segment)[0].1;
        let moof = &segment[..moof_size];
        let traf_index = moof.windows(4).position(|w| w == b"traf").unwrap();
        let traf = &moof[traf_index - 4..];
        let trun_index = traf.windows(4).position(|w| w == b"trun").unwrap();
        let trun = &traf[trun_index - 4..];
        let payload = &trun[12..];
        assert_eq!(
            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
            1
        );
        let data_offset = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
        assert_eq!(
            data_offset as usize,
            moof_size + 8,
            "data offset must point at the first sample inside mdat"
        );
        assert_eq!(
            u32::from_be_bytes([payload[12], payload[13], payload[14], payload[15]]),
            1500
        );
        assert_eq!(
            u32::from_be_bytes([payload[16], payload[17], payload[18], payload[19]]),
            7
        );
    }

    #[test]
    fn avcc_sample_prefixes_the_length() {
        assert_eq!(avcc_sample(&[0x65, 0x01]), vec![0, 0, 0, 2, 0x65, 0x01]);
    }
}
