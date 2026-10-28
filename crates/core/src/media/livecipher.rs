use md5::{Digest, Md5};

pub const DHAV_HEADER: usize = 16;
pub const KEY_BYTES: usize = 128;
pub const MASK_WINDOW: usize = 128;
pub const MATERIAL_BYTES: usize = 40;
pub const MATERIAL_MIN: usize = 16;
pub const MD5_INPUT: usize = 8;
pub const MD5_STRIDE: usize = 4;

const SALT_MOD: u32 = 253;

pub struct LiveCipher {
    key: [u8; KEY_BYTES],
}

impl LiveCipher {
    pub fn derive(material: &str) -> Option<Self> {
        let raw = material.as_bytes();
        if raw.len() < MATERIAL_MIN {
            return None;
        }
        let mut buf = [0u8; MATERIAL_BYTES];
        let take = raw.len().min(MATERIAL_BYTES);
        buf[..take].copy_from_slice(&raw[..take]);
        let mut key = [0u8; KEY_BYTES];
        for round in 0..KEY_BYTES / 16 {
            let start = round * MD5_STRIDE;
            let digest = Md5::digest(&buf[start..start + MD5_INPUT]);
            key[round * 16..round * 16 + 16].copy_from_slice(&digest);
        }
        Some(Self { key })
    }

    pub fn from_key(key: [u8; KEY_BYTES]) -> Self {
        Self { key }
    }

    pub fn key(&self) -> &[u8; KEY_BYTES] {
        &self.key
    }

    pub fn frame_len(declared: u32) -> u32 {
        declared.wrapping_add(DHAV_HEADER as u32)
    }

    pub fn mask(&self, payload: &mut [u8], frame_len: u32) -> usize {
        let limit = (frame_len as usize)
            .min(MASK_WINDOW)
            .min(payload.len());
        let salt = (frame_len % SALT_MOD) as u8;
        for i in 0..limit {
            let source = (self.key[i] as u32 + frame_len) % MASK_WINDOW as u32;
            payload[i] ^= self.key[source as usize] ^ salt;
        }
        limit
    }

    pub fn unmask(&self, payload: &mut [u8], frame_len: u32) -> usize {
        self.mask(payload, frame_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATERIAL: &str = "zha4072-synthetic-vekey-material-0001";

    const KEY_HEX: &str = "f6b6453805f060608201ee5887d5c07e\
d5cd4f10d7fdb2a64b304c0439aaeef2\
ee8b8e3857b638c1355a9741bfcf48e3\
51e22dc2eac3de98ecc50d3a5ef05ece\
2ea1e4ab39e282441e31b4e0f4db6af3\
a37455a7b05633f5f60bee17d80aa755\
eec34d804c9ce6c89cff596be555e6a4\
218ba077697ac3f1636de9802d10e942";

    const MASKED_HEX: &str = "2bd00f7e9a98545d4cc7f884b9ba8ef4\
a7c89c3e1f0b0132e4d4595fbe9b74e1\
42b6575e8fcc4b288133b00c8c53fb63\
d882d965de9923f9fe8c4b5c09001b23\
d570f09a960efa0c0beb68795a0d844c\
b907958839f88edc533e2809539a4578\
0255f3555f31566a1cd44e38c5da8ebe\
b96638101a90a2ba640a4cfddfd85091\
8b9299a0a7aeb5bcc3cad1d8dfe6edf4\
fb020910171e252c333a41484f565d64\
6b727980878e959ca3aab1b8bfc6cdd4\
dbe2e9f0f7fe050c131a21282f363d44\
4b525960676e757c";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
            .collect()
    }

    fn synthetic_payload() -> Vec<u8> {
        (0..200u32).map(|i| ((i * 7 + 11) & 0xff) as u8).collect()
    }

    #[test]
    fn derive_matches_the_pinned_vector() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        assert_eq!(cipher.key().to_vec(), hex(KEY_HEX));
    }

    #[test]
    fn mask_matches_the_pinned_vector() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let mut payload = synthetic_payload();
        let frame_len = LiveCipher::frame_len(4096);
        assert_eq!(frame_len, 4112);
        assert_eq!(cipher.mask(&mut payload, frame_len), MASK_WINDOW);
        assert_eq!(payload, hex(MASKED_HEX));
    }

    #[test]
    fn mask_is_its_own_inverse() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let plain = synthetic_payload();
        let mut payload = plain.clone();
        cipher.mask(&mut payload, 4112);
        cipher.unmask(&mut payload, 4112);
        assert_eq!(payload, plain);
    }

    #[test]
    fn mask_leaves_everything_past_the_window_alone() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let plain = synthetic_payload();
        let mut payload = plain.clone();
        cipher.mask(&mut payload, 4112);
        assert_eq!(&payload[MASK_WINDOW..], &plain[MASK_WINDOW..]);
        assert_ne!(&payload[..MASK_WINDOW], &plain[..MASK_WINDOW]);
    }

    #[test]
    fn a_frame_shorter_than_the_window_is_masked_in_full() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let mut payload = vec![0u8; 8];
        assert_eq!(cipher.mask(&mut payload, 32), 8);
    }

    #[test]
    fn a_short_frame_length_narrows_the_window() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let mut payload = synthetic_payload();
        assert_eq!(cipher.mask(&mut payload, 6), 6);
    }

    #[test]
    fn derive_rejects_material_below_the_minimum() {
        assert!(LiveCipher::derive("0123456789abcde").is_none());
        assert!(LiveCipher::derive("0123456789abcdef").is_some());
    }

    #[test]
    fn material_past_the_forty_byte_prefix_does_not_reach_the_digest() {
        let head = format!("{MATERIAL}abc");
        assert_eq!(head.len(), MATERIAL_BYTES);
        let first = LiveCipher::derive(&format!("{head}xxxx")).expect("first");
        let second = LiveCipher::derive(&format!("{head}yyyy")).expect("second");
        assert_eq!(first.key(), second.key());
    }

    #[test]
    fn frame_len_adds_the_dhav_header() {
        assert_eq!(LiveCipher::frame_len(36870), 36886);
        assert_eq!(LiveCipher::frame_len(0), DHAV_HEADER as u32);
    }
}
