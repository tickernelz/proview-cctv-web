use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Block};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

pub const PORTAL_KEY_HEX: &str = "d86d7bab3d6ac01ad9dc6a897652f2d2";
const BLOCK: usize = 16;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DigestError {
    #[error("field is not valid base64")]
    Base64,
    #[error("field is not a whole number of AES blocks")]
    Length,
    #[error("field padding is invalid")]
    Padding,
    #[error("field is not valid UTF-8")]
    Utf8,
}

fn cipher() -> Aes128 {
    let mut key = [0u8; BLOCK];
    for (index, slot) in key.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&PORTAL_KEY_HEX[index * 2..index * 2 + 2], 16)
            .expect("portal key is valid hex");
    }
    Aes128::new_from_slice(&key).expect("aes-128 accepts a 16 byte key")
}

pub fn digest_of(value: &str) -> String {
    STANDARD.encode(digest_bytes(value.as_bytes()))
}

fn digest_bytes(plain: &[u8]) -> Vec<u8> {
    let mut buffer = plain.to_vec();
    let pad = BLOCK - buffer.len() % BLOCK;
    buffer.extend(std::iter::repeat_n(pad as u8, pad));
    let cipher = cipher();
    for chunk in buffer.chunks_mut(BLOCK) {
        cipher.encrypt_block(Block::from_mut_slice(chunk));
    }
    buffer
}

pub fn undigest_of(value: &str) -> Result<String, DigestError> {
    let bytes = STANDARD
        .decode(value.trim())
        .map_err(|_| DigestError::Base64)?;
    plain_of(&bytes)
}

fn plain_of(bytes: &[u8]) -> Result<String, DigestError> {
    if bytes.is_empty() || bytes.len() % BLOCK != 0 {
        return Err(DigestError::Length);
    }
    let mut buffer = bytes.to_vec();
    let cipher = cipher();
    for chunk in buffer.chunks_mut(BLOCK) {
        cipher.decrypt_block(Block::from_mut_slice(chunk));
    }
    let pad = *buffer.last().ok_or(DigestError::Length)? as usize;
    if pad == 0 || pad > BLOCK || pad > buffer.len() {
        return Err(DigestError::Padding);
    }
    let split = buffer.len() - pad;
    if buffer[split..].iter().any(|byte| *byte as usize != pad) {
        return Err(DigestError::Padding);
    }
    buffer.truncate(split);
    String::from_utf8(buffer).map_err(|_| DigestError::Utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_cryptojs_reference() {
        assert_eq!(digest_of("hunter2"), "I7TeQB7MYIgzzZViECWsGg==");
    }

    #[test]
    fn pads_a_whole_block() {
        let digits = digest_of("0123456789abcdef");
        assert_eq!(STANDARD.decode(digits).unwrap().len(), 32);
    }

    #[test]
    fn round_trips_utf8() {
        for value in ["hunter2", "", "0123456789abcdef", "kata sandi dengan spasi"] {
            assert_eq!(undigest_of(&digest_of(value)).as_deref(), Ok(value));
        }
    }

    #[test]
    fn rejects_broken_padding() {
        let mut bytes = digest_bytes(b"hunter2");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x7f;
        assert_eq!(plain_of(&bytes), Err(DigestError::Padding));
    }

    #[test]
    fn rejects_short_ciphertext() {
        assert_eq!(undigest_of("AAAA"), Err(DigestError::Length));
        assert_eq!(undigest_of("not base64!"), Err(DigestError::Base64));
    }
}
