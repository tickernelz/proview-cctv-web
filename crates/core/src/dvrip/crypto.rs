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
        let modulus = "d0b8599c39263059f28c105d1fb17c2390c192cfd3ac94af0f21ddb66cad4a268d116ece1738f7d93d9c172411e20b8f6b0d549b6f03675a1600a35a099950d836f675cc81e74ef5e8e25d940ed904759531985d5d9dc9f81818e811892f902bd23f0824128b2f330c5c7fd0a6a3a4506513270e269e0d37f2a74de452e6b439";
        let sealed = communicate_key(modulus, "010001", "iq5H3765j5t04mO6").expect("sealed");
        assert_eq!(modulus.len(), 256);
        assert_eq!(sealed.len(), 256);
        assert!(sealed.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_broken_modulus_is_refused() {
        assert!(communicate_key("not-hex", "010001", "abcdefghijklmnop").is_err());
    }
}
