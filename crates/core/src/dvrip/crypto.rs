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
}
