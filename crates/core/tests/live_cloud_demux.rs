use std::path::PathBuf;
use std::process::Command;

use proview_core::media::fmp4::{media_segment, Sample, TIMESCALE};
use proview_core::media::hevc;
use proview_core::media::live::{LiveDemux, LiveStats};
use proview_core::media::nal::split_annexb;

const MATERIAL: &str = "zha4084-live-cloud-fixture-material-0001";
const KEYED: &str = "cloud-keyed-1280x720.bodies";
const PLAIN: &str = "cloud-plain-1280x720.bodies";

fn report(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn fixture(name: &str) -> Vec<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("live-cloud")
        .join(name);
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut bodies = Vec::new();
    let mut at = 0usize;
    while at + 4 <= raw.len() {
        let len = u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]) as usize;
        at += 4;
        assert!(at + len <= raw.len(), "truncated body at {at}");
        bodies.push(raw[at..at + len].to_vec());
        at += len;
    }
    bodies
}

fn drain(demux: &mut LiveDemux, bodies: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for body in bodies {
        out.extend(demux.push(body));
    }
    out.extend(demux.finish());
    out
}

fn box_bytes(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn full_box(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + payload.len());
    body.push(version);
    body.extend_from_slice(&[(flags >> 16) as u8, (flags >> 8) as u8, flags as u8]);
    body.extend_from_slice(payload);
    box_bytes(kind, &body)
}

fn hevc_init(width: u32, height: u32, hvcc: &[u8]) -> Vec<u8> {
    let mut ftyp = Vec::new();
    ftyp.extend_from_slice(b"isom");
    ftyp.extend_from_slice(&512u32.to_be_bytes());
    ftyp.extend_from_slice(b"isomiso5hvc1mp41");
    let ftyp = box_bytes(b"ftyp", &ftyp);

    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&0u32.to_be_bytes());
    mvhd.extend_from_slice(&0u32.to_be_bytes());
    mvhd.extend_from_slice(&1000u32.to_be_bytes());
    mvhd.extend_from_slice(&0u32.to_be_bytes());
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
    tkhd.extend_from_slice(&0u32.to_be_bytes());
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
    mdhd.extend_from_slice(&0u32.to_be_bytes());
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
    let dref = full_box(b"dref", 0, 0, &dref);
    let dinf = box_bytes(b"dinf", &dref);

    let mut hvc1 = Vec::new();
    hvc1.extend_from_slice(&[0u8; 6]);
    hvc1.extend_from_slice(&1u16.to_be_bytes());
    hvc1.extend_from_slice(&[0u8; 16]);
    hvc1.extend_from_slice(&(width as u16).to_be_bytes());
    hvc1.extend_from_slice(&(height as u16).to_be_bytes());
    hvc1.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    hvc1.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    hvc1.extend_from_slice(&0u32.to_be_bytes());
    hvc1.extend_from_slice(&1u16.to_be_bytes());
    hvc1.extend_from_slice(&[0u8; 32]);
    hvc1.extend_from_slice(&0x0018u16.to_be_bytes());
    hvc1.extend_from_slice(&0xffffu16.to_be_bytes());
    hvc1.extend(box_bytes(b"hvcC", hvcc));
    let hvc1 = box_bytes(b"hvc1", &hvc1);

    let mut stsd = Vec::new();
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend(hvc1);
    let stsd = full_box(b"stsd", 0, 0, &stsd);

    let stts = full_box(b"stts", 0, 0, &0u32.to_be_bytes());
    let stsc = full_box(b"stsc", 0, 0, &0u32.to_be_bytes());
    let stsz = full_box(b"stsz", 0, 0, &[0u8; 8]);
    let stco = full_box(b"stco", 0, 0, &0u32.to_be_bytes());

    let mut stbl = Vec::new();
    stbl.extend(stsd);
    stbl.extend(stts);
    stbl.extend(stsc);
    stbl.extend(stsz);
    stbl.extend(stco);
    let stbl = box_bytes(b"stbl", &stbl);

    let mut minf = Vec::new();
    minf.extend(vmhd);
    minf.extend(dinf);
    minf.extend(stbl);
    let minf = box_bytes(b"minf", &minf);

    let mut mdia = Vec::new();
    mdia.extend(mdhd);
    mdia.extend(hdlr);
    mdia.extend(minf);
    let mdia = box_bytes(b"mdia", &mdia);

    let mut trak = Vec::new();
    trak.extend(tkhd);
    trak.extend(mdia);
    let trak = box_bytes(b"trak", &trak);

    let mut trex = Vec::new();
    trex.extend_from_slice(&1u32.to_be_bytes());
    trex.extend_from_slice(&1u32.to_be_bytes());
    trex.extend_from_slice(&0u32.to_be_bytes());
    trex.extend_from_slice(&0u32.to_be_bytes());
    trex.extend_from_slice(&0u32.to_be_bytes());
    let trex = full_box(b"trex", 0, 0, &trex);
    let mvex = box_bytes(b"mvex", &trex);

    let mut moov = Vec::new();
    moov.extend(mvhd);
    moov.extend(trak);
    moov.extend(mvex);
    let moov = box_bytes(b"moov", &moov);

    let mut out = ftyp;
    out.extend(moov);
    out
}

struct Unit {
    nals: Vec<Vec<u8>>,
    keyframe: bool,
}

fn access_units(es: &[u8]) -> Vec<Unit> {
    let units = split_annexb(es);
    let mut out: Vec<Unit> = Vec::new();
    for unit in units {
        let nal = unit.data.to_vec();
        let kind = unit.h265_type().unwrap_or(255);
        if kind == 35 || out.is_empty() {
            out.push(Unit {
                nals: Vec::new(),
                keyframe: false,
            });
        }
        let current = out.last_mut().expect("a unit");
        if kind != 35 {
            if (16..=21).contains(&kind) {
                current.keyframe = true;
            }
            current.nals.push(nal);
        }
    }
    out
}

fn fmp4(es: &[u8], fps: u32) -> Vec<u8> {
    let units = access_units(es);
    let param = |kind: u8| -> Vec<u8> {
        units
            .iter()
            .flat_map(|unit| unit.nals.iter())
            .find(|nal| hevc::nal_type(nal) == Some(kind))
            .unwrap_or_else(|| panic!("no NAL {kind}"))
            .clone()
    };
    let vps = param(32);
    let sps = param(33);
    let pps = param(34);
    let parsed = hevc::parse_sps(&sps).expect("SPS");
    let hvcc = hevc::build_hvcc(&parsed, &vps, &sps, &pps);

    let mut out = hevc_init(parsed.width, parsed.height, &hvcc);
    let duration = TIMESCALE / fps.max(1);
    let mut groups: Vec<Vec<Sample>> = Vec::new();
    for unit in &units {
        if unit.nals.is_empty() {
            continue;
        }
        if unit.keyframe || groups.is_empty() {
            groups.push(Vec::new());
        }
        groups.last_mut().expect("a group").push(Sample {
            data: hevc::avcc(&unit.nals),
            duration,
            keyframe: unit.keyframe,
        });
    }
    let mut base = 0u64;
    for (index, group) in groups.iter().enumerate() {
        out.extend(media_segment(index as u32 + 1, base, group));
        base += duration as u64 * group.len() as u64;
    }
    out
}

fn ffprobe(path: &std::path::Path, extra: &[&str]) -> Option<String> {
    let output = Command::new("ffprobe")
        .args(["-v", "error"])
        .args(extra)
        .arg("-show_entries")
        .arg("stream=codec_name,width,height,nb_read_frames")
        .arg("-of")
        .arg("default=nw=1")
        .arg(path)
        .output()
        .ok()?;
    assert!(
        output.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[test]
fn the_keyed_cloud_bodies_demux_to_the_plain_capture() {
    let plain = drain(&mut LiveDemux::new(), &fixture(PLAIN));
    let mut keyed = LiveDemux::keyed(MATERIAL).expect("material");
    let keyed_es = drain(&mut keyed, &fixture(KEYED));

    assert_eq!(
        &plain[..6],
        &[0, 0, 0, 1, 0x46, 0x01],
        "an unmasked capture opens on a four-byte start code and an access-unit delimiter"
    );
    assert!(
        plain.windows(6).any(|w| w == [0, 0, 0, 1, 0x40, 0x01]),
        "the capture carries the VPS the relay would have masked"
    );
    assert!(
        keyed_es.starts_with(&plain),
        "the masked capture must demux to the plain byte stream, {} vs {}",
        keyed_es.len(),
        plain.len()
    );

    let stats: &LiveStats = keyed.stats();
    assert_eq!(stats.keyframes, 2);
    assert_eq!(stats.deltas, 6);
    assert_eq!(stats.markers, 3);
    assert_eq!(stats.padding, 6);
    assert_eq!(stats.continuations, 4);
    assert_eq!(
        stats.dropped, 1,
        "the capture ends inside a keyframe, which is dropped whole"
    );
    assert_eq!(stats.elementary_bytes, keyed_es.len() as u64);
    let source = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("live-cloud")
            .join("source-1280x720.hevc"),
    )
    .expect("source stream");
    assert_eq!(
        keyed_es, source,
        "the keyed capture demuxes back to the encoded stream byte for byte"
    );

    report(&format!(
        "live_cloud keyed: bodies={} bytes_in={} es={} keyframes={} deltas={} dropped={} padding={} markers={} continuations={}",
        fixture(KEYED).len(),
        fixture(KEYED).iter().map(Vec::len).sum::<usize>(),
        keyed_es.len(),
        2,
        6,
        1,
        6,
        3,
        4
    ));
}

#[test]
fn the_demuxed_cloud_stream_becomes_an_fmp4_that_ffprobe_reads() {
    let mut keyed = LiveDemux::keyed(MATERIAL).expect("material");
    let es = drain(&mut keyed, &fixture(KEYED));
    let mp4 = fmp4(&es, 12);
    let path = std::env::temp_dir().join(format!(
        "proview-live-cloud-{}-{}.mp4",
        std::process::id(),
        es.len()
    ));
    std::fs::write(&path, &mp4).expect("write fmp4");

    let probe = match ffprobe(&path, &["-count_frames"]) {
        Some(text) => text,
        None => {
            report(&format!(
                "live_cloud ffprobe skipped: ffprobe is not installed, wrote {} B of fMP4 for the {} B elementary stream",
                mp4.len(),
                es.len()
            ));
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);

    assert!(probe.contains("codec_name=hevc"), "ffprobe read {probe}");
    assert!(probe.contains("width=1280"), "ffprobe read {probe}");
    assert!(probe.contains("height=720"), "ffprobe read {probe}");
    assert!(probe.contains("nb_read_frames=8"), "ffprobe read {probe}");

    report(&format!(
        "live_cloud fmp4: es={} mp4={} probe={}",
        es.len(),
        mp4.len(),
        probe.trim().replace('\n', " ")
    ));
}
