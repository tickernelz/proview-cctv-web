use md5::{Digest, Md5};

pub const KEY_BYTES: usize = 128;

const DHAV_HEADER: u32 = 16;
const MASK_WINDOW: usize = 128;
const MATERIAL_BYTES: usize = 40;
const MATERIAL_MIN: usize = 16;
const MD5_INPUT: usize = 8;
const MD5_STRIDE: usize = 4;
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

    pub fn mask(&self, payload: &mut [u8], declared: u32) -> usize {
        let frame_len = declared.wrapping_add(DHAV_HEADER);
        let limit = (frame_len as usize).min(MASK_WINDOW).min(payload.len());
        let salt = (frame_len % SALT_MOD) as u8;
        let shift = frame_len % MASK_WINDOW as u32;
        for (i, byte) in payload.iter_mut().enumerate().take(limit) {
            let source = ((self.key[i] as u32 % MASK_WINDOW as u32) + shift) % MASK_WINDOW as u32;
            *byte ^= self.key[source as usize] ^ salt;
        }
        limit
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

    const SHORT_KEY_HEAD_HEX: &str = "2e9ec317e197819358fbc43afca7d837\
b69ed510e28e758804c5ef69609e7907\
3d6c946ea0da9f49ee4e017a66732f51\
5719788e17b708195c9db73b9467e5a7";

    const ZERO_PADDED_TAIL_HEX: &str = "7dea362b3fac8e00956a4952a3d4f474";

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
    fn a_short_material_leaves_the_tail_of_the_key_on_zero_bytes() {
        let cipher = LiveCipher::derive("0123456789abcdef").expect("material");
        assert_eq!(cipher.key()[..64].to_vec(), hex(SHORT_KEY_HEAD_HEX));
        for round in 4..KEY_BYTES / 16 {
            let start = round * 16;
            assert_eq!(
                cipher.key()[start..start + 16].to_vec(),
                hex(ZERO_PADDED_TAIL_HEX)
            );
        }
    }

    #[test]
    fn mask_matches_the_pinned_vector() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let mut payload = synthetic_payload();
        assert_eq!(cipher.mask(&mut payload, 4096), MASK_WINDOW);
        assert_eq!(payload, hex(MASKED_HEX));
    }

    #[test]
    fn mask_is_its_own_inverse() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let plain = synthetic_payload();
        let mut payload = plain.clone();
        cipher.mask(&mut payload, 4096);
        cipher.mask(&mut payload, 4096);
        assert_eq!(payload, plain);
    }

    #[test]
    fn mask_leaves_everything_past_the_window_alone() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let plain = synthetic_payload();
        let mut payload = plain.clone();
        cipher.mask(&mut payload, 4096);
        assert_eq!(&payload[MASK_WINDOW..], &plain[MASK_WINDOW..]);
        assert_ne!(&payload[..MASK_WINDOW], &plain[..MASK_WINDOW]);
    }

    #[test]
    fn the_window_is_the_frame_length_clamped_to_the_payload() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let mut payload = vec![0u8; 200];
        assert_eq!(cipher.mask(&mut payload, 0), 16);
        assert_eq!(payload[16], 0);

        let mut small = vec![0u8; 8];
        assert_eq!(cipher.mask(&mut small, 4096), 8);

        let mut wide = vec![0u8; 400];
        assert_eq!(cipher.mask(&mut wide, 4096), 128);
    }

    #[test]
    fn a_frame_length_near_the_u32_ceiling_does_not_overflow() {
        let cipher = LiveCipher::derive(MATERIAL).expect("material");
        let plain = synthetic_payload();
        let mut payload = plain.clone();
        assert_eq!(
            cipher.mask(&mut payload, u32::MAX - DHAV_HEADER),
            MASK_WINDOW
        );
        cipher.mask(&mut payload, u32::MAX - DHAV_HEADER);
        assert_eq!(payload, plain);

        let mut wrapped = synthetic_payload();
        assert_eq!(
            cipher.mask(&mut wrapped, u32::MAX),
            DHAV_HEADER as usize - 1
        );
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
}
