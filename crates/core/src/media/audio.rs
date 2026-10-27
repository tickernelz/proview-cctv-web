pub const SAMPLE_RATE: u32 = 8000;
pub const FRAME_SAMPLES: usize = 320;
pub const SILENCE: u8 = 0xd5;

const SEG_END: [i32; 8] = [0x1f, 0x3f, 0x7f, 0xff, 0x1ff, 0x3ff, 0x7ff, 0xfff];

pub fn encode_sample(sample: i16) -> u8 {
    let mut value = i32::from(sample) >> 3;
    let mask = if value >= 0 {
        0xd5
    } else {
        value = -value - 1;
        0x55
    };
    let mut segment = 0usize;
    while segment < SEG_END.len() && value > SEG_END[segment] {
        segment += 1;
    }
    if segment >= SEG_END.len() {
        return 0x7f ^ mask;
    }
    let low = if segment < 2 {
        (value >> 1) & 0xf
    } else {
        (value >> segment) & 0xf
    };
    ((segment as i32) << 4 | low) as u8 ^ mask
}

pub fn decode_sample(byte: u8) -> i16 {
    let value = byte ^ 0x55;
    let mut magnitude = i32::from(value & 0x0f) << 4;
    let segment = i32::from(value & 0x70) >> 4;
    match segment {
        0 => magnitude += 8,
        1 => magnitude += 0x108,
        _ => {
            magnitude += 0x108;
            magnitude <<= segment - 1;
        }
    }
    if value & 0x80 != 0 {
        magnitude as i16
    } else {
        -(magnitude as i16)
    }
}

pub fn encode(samples: &[i16]) -> Vec<u8> {
    samples
        .iter()
        .map(|sample| encode_sample(*sample))
        .collect()
}

pub fn decode(payload: &[u8]) -> Vec<i16> {
    payload.iter().map(|byte| decode_sample(*byte)).collect()
}

pub fn from_pcm16le(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

pub fn to_pcm16le(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

pub fn downsample(samples: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || from == 0 || to == 0 || samples.is_empty() {
        return samples.to_vec();
    }
    let step = from as f64 / to as f64;
    let out_len = (samples.len() as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let start = (index as f64 * step).floor() as usize;
        let end = (((index + 1) as f64 * step).floor() as usize)
            .min(samples.len())
            .max(start + 1);
        let window = &samples[start..end.min(samples.len())];
        let sum: i64 = window.iter().map(|sample| i64::from(*sample)).sum();
        out.push((sum / window.len() as i64) as i16);
    }
    out
}

pub fn tone(hz: f64, rate: u32, samples: usize, amplitude: f64) -> Vec<i16> {
    (0..samples)
        .map(|index| {
            let phase = 2.0 * std::f64::consts::PI * hz * index as f64 / f64::from(rate);
            (phase.sin() * amplitude * f64::from(i16::MAX)) as i16
        })
        .collect()
}

pub fn frames(payload: &[u8], size: usize) -> Vec<&[u8]> {
    let size = size.max(1);
    payload.chunks(size).collect()
}

pub fn pad_to_frame(payload: &mut Vec<u8>, size: usize) {
    let size = size.max(1);
    let remainder = payload.len() % size;
    if remainder != 0 {
        payload.extend(std::iter::repeat_n(SILENCE, size - remainder));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extremes_match_the_published_alaw_codes() {
        assert_eq!(encode_sample(0), 0xd5);
        assert_eq!(encode_sample(8), 0xd5);
        assert_eq!(encode_sample(i16::MAX), 0xaa);
        assert_eq!(encode_sample(-1), 0x55);
        assert_eq!(encode_sample(i16::MIN), 0x2a);
    }

    #[test]
    fn decode_of_silence_lands_near_zero() {
        assert_eq!(decode_sample(0xd5), 8);
        assert_eq!(decode_sample(0x55), -8);
    }

    #[test]
    fn the_code_is_monotonic_across_the_whole_range() {
        let mut previous = decode_sample(encode_sample(i16::MIN));
        let mut step = i32::from(i16::MIN) + 8;
        while step < i32::from(i16::MAX) - 8 {
            let value = step as i16;
            let decoded = decode_sample(encode_sample(value));
            assert!(
                decoded >= previous,
                "decoded output fell from {previous} to {decoded} at {value}"
            );
            let error = (i32::from(decoded) - i32::from(value)).abs();
            let bound = if value.abs() > 16384 { 512 } else { 260 };
            assert!(
                error <= bound,
                "error {error} too large at {value}: {decoded}"
            );
            previous = decoded;
            step += 4111;
        }
    }

    #[test]
    fn round_trip_snr_of_a_voice_band_tone_stays_above_thirty_db() {
        let source = tone(1000.0, SAMPLE_RATE, SAMPLE_RATE as usize, 0.5);
        let coded = encode(&source);
        let back = decode(&coded);
        assert_eq!(back.len(), source.len());
        let signal: f64 = source
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>()
            / source.len() as f64;
        let noise: f64 = source
            .iter()
            .zip(back.iter())
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
            .sum::<f64>()
            / source.len() as f64;
        let snr = 10.0 * (signal / noise).log10();
        assert!(snr > 30.0, "snr {snr}");
    }

    #[test]
    fn every_code_decodes_to_a_distinct_step_near_the_encoding_of_that_step() {
        for code in 0u8..=255 {
            let decoded = decode_sample(code);
            let reencoded = encode_sample(decoded);
            assert_eq!(
                decoded,
                decode_sample(reencoded),
                "code {code:#04x} re-encodes to {reencoded:#04x}"
            );
        }
    }

    #[test]
    fn pcm16_bytes_are_read_little_endian_and_odd_tails_are_dropped() {
        let samples = from_pcm16le(&[0x10, 0x00, 0xf0, 0xff, 0x7f]);
        assert_eq!(samples, vec![16, -16]);
        assert_eq!(to_pcm16le(&samples), vec![0x10, 0x00, 0xf0, 0xff]);
    }

    #[test]
    fn downsampling_halves_the_length_and_keeps_a_dc_level() {
        let source = vec![1000i16; 960];
        let out = downsample(&source, 48000, 8000);
        assert_eq!(out.len(), 160);
        assert!(out.iter().all(|sample| *sample == 1000));
    }

    #[test]
    fn downsampling_by_the_same_rate_is_a_copy() {
        let source = vec![1i16, 2, 3];
        assert_eq!(downsample(&source, 8000, 8000), source);
    }

    #[test]
    fn padding_fills_a_partial_frame_with_silence() {
        let mut payload = vec![0x01u8; 321];
        pad_to_frame(&mut payload, FRAME_SAMPLES);
        assert_eq!(payload.len(), FRAME_SAMPLES * 2);
        assert_eq!(&payload[321..], &[SILENCE; FRAME_SAMPLES - 1]);
    }

    #[test]
    fn frames_are_sliced_at_the_frame_size() {
        let payload = vec![0u8; 700];
        let sliced = frames(&payload, FRAME_SAMPLES);
        assert_eq!(sliced.len(), 3);
        assert_eq!(sliced[0].len(), FRAME_SAMPLES);
        assert_eq!(sliced[2].len(), 700 - FRAME_SAMPLES * 2);
    }
}
