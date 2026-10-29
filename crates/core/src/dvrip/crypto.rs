use md5::{Digest, Md5};

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn sofia_hash(input: &str) -> String {
    let digest = Md5::digest(input.as_bytes());
    let mut hash = String::with_capacity(8);
    for pair in digest.chunks_exact(2) {
        let index = (pair[0] as u16 + pair[1] as u16) % ALPHABET.len() as u16;
        hash.push(ALPHABET[index as usize] as char);
    }
    hash
}

use aes::cipher::{BlockDecrypt, KeyInit};
use aes::{Aes128, Block};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

pub const LOGIN_TOKEN_LENGTH: usize = 16;

pub fn login_token(serial: &str, device_token: &str) -> Option<String> {
    let serial = serial.trim();
    if serial.len() < LOGIN_TOKEN_LENGTH {
        return None;
    }
    let mut key = [0u8; LOGIN_TOKEN_LENGTH];
    key.copy_from_slice(&serial.as_bytes()[..LOGIN_TOKEN_LENGTH]);
    key.reverse();
    let mut plain = STANDARD.decode(device_token.trim()).ok()?;
    if plain.is_empty() || plain.len() % LOGIN_TOKEN_LENGTH != 0 {
        return None;
    }
    let cipher = Aes128::new_from_slice(&key).ok()?;
    for block in plain.chunks_mut(LOGIN_TOKEN_LENGTH) {
        cipher.decrypt_block(Block::from_mut_slice(block));
    }
    let pad = usize::from(*plain.last()?);
    if pad == 0 || pad > LOGIN_TOKEN_LENGTH {
        return None;
    }
    if plain[plain.len() - pad..]
        .iter()
        .any(|byte| usize::from(*byte) != pad)
    {
        return None;
    }
    plain.truncate(plain.len() - pad);
    if plain.is_empty() || !plain.iter().all(|byte| byte.is_ascii_graphic()) {
        return None;
    }
    String::from_utf8(plain).ok()
}

use rsa::pkcs1v15::Pkcs1v15Encrypt;
use rsa::{BigUint, RsaPublicKey};

pub fn communicate_key(
    modulus_hex: &str,
    exponent_hex: &str,
    session: &str,
) -> Result<String, String> {
    let modulus = BigUint::parse_bytes(modulus_hex.trim().as_bytes(), 16)
        .ok_or_else(|| "the device modulus is not hex".to_string())?;
    let exponent = BigUint::parse_bytes(exponent_hex.trim().as_bytes(), 16)
        .ok_or_else(|| "the device exponent is not hex".to_string())?;
    let key = RsaPublicKey::new(modulus, exponent)
        .map_err(|error| format!("the device key does not parse: {error}"))?;
    let sealed = key
        .encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, session.as_bytes())
        .map_err(|error| format!("the session key does not seal: {error}"))?;
    let mut out = String::with_capacity(sealed.len() * 2);
    for byte in sealed {
        out.push_str(&format!("{byte:02X}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_known_vector() {
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn sofia_hash_matches_the_empty_password_vector() {
        assert_eq!(sofia_hash(""), "tlJwpbo6");
    }

    #[test]
    fn sofia_hash_is_eight_alphanumeric_characters() {
        for password in ["admin", "hunter2", "kamera-rumah", "correct horse"] {
            let hash = sofia_hash(password);
            assert_eq!(hash.len(), 8, "{password}");
            assert!(hash.chars().all(|c| c.is_ascii_alphanumeric()), "{hash}");
        }
    }

    #[test]
    fn sofia_hash_folds_adjacent_md5_bytes_modulo_the_alphabet() {
        let digest = Md5::digest(b"admin");
        let expected: String = digest
            .chunks_exact(2)
            .map(|pair| ALPHABET[((pair[0] as u16 + pair[1] as u16) % 62) as usize] as char)
            .collect();
        assert_eq!(sofia_hash("admin"), expected);
        assert_ne!(sofia_hash("admin"), md5_hex("admin")[..8].to_string());
    }
    #[test]
    fn the_communicate_key_is_one_rsa_block_of_hex() {
        let modulus = "b4a1c0d54f0f0b1e4a2f6d3c9e8b7a6918273645f0e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f";
        let sealed = communicate_key(modulus, "010001", "iq5H3765j5t04mO6").expect("sealed");
        assert_eq!(sealed.len(), modulus.len(), "one block, one modulus wide");
        assert!(sealed.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_broken_modulus_is_refused() {
        assert!(communicate_key("not-hex", "010001", "abcdefghijklmnop").is_err());
    }

    fn seal_login_token(serial: &str, token: &str) -> String {
        use aes::cipher::BlockEncrypt;
        let mut key = [0u8; 16];
        key.copy_from_slice(&serial.as_bytes()[..16]);
        key.reverse();
        let cipher = Aes128::new_from_slice(&key).expect("aes-128 key");
        let mut buffer = token.as_bytes().to_vec();
        let pad = 16 - buffer.len() % 16;
        buffer.extend(std::iter::repeat_n(pad as u8, pad));
        for block in buffer.chunks_mut(16) {
            cipher.encrypt_block(Block::from_mut_slice(block));
        }
        STANDARD.encode(buffer)
    }

    #[test]
    fn the_login_token_is_the_device_token_opened_with_the_serial() {
        let serial = "0123456789abcdefghij";
        let token = "F0+7ghjHWzNsktOE";
        let sealed = seal_login_token(serial, token);
        assert_eq!(sealed.len(), 44);
        assert_eq!(login_token(serial, &sealed).as_deref(), Some(token));
    }

    #[test]
    fn a_login_token_of_another_length_survives_the_padding() {
        let serial = "0123456789abcdefghij";
        let token = "abcdefghijklmnopqrstuvwx";
        assert_eq!(
            login_token(serial, &seal_login_token(serial, token)).as_deref(),
            Some(token)
        );
    }

    #[test]
    fn the_token_derivation_refuses_what_it_cannot_open() {
        let serial = "0123456789abcdefghij";
        assert!(login_token("short", "AAAAAAAAAAAAAAAAAAAAAA==").is_none());
        assert!(login_token(serial, "not base64!").is_none());
        assert!(login_token(serial, &STANDARD.encode([0u8; 16])).is_none());
        assert!(login_token(serial, &seal_login_token(serial, "ab\u{1}cd")).is_none());
    }
}
