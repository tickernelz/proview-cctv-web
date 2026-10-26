use md5::{Digest, Md5};

pub fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn login_hash(user: &str, password: &str) -> String {
    md5_hex(&format!("{user}{password}"))
}

pub fn realm_hash(user: &str, realm: &str, password: &str) -> String {
    md5_hex(&format!("{user}{realm}{password}"))
}

pub fn realm_hash_with_legacy_secret(user: &str, realm: &str, secret_hash: &str) -> String {
    md5_hex(&format!("{user}{realm}{secret_hash}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_known_vector() {
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn login_hash_matches_known_vector() {
        assert_eq!(login_hash("admin", "admin"), md5_hex("adminadmin"));
    }

    #[test]
    fn realm_hash_uses_the_realm_in_the_middle() {
        assert_eq!(
            realm_hash("admin", "abcd1234", "admin"),
            md5_hex("adminabcd1234admin")
        );
        assert_ne!(
            realm_hash("admin", "abcd1234", "admin"),
            login_hash("admin", "admin")
        );
    }

    #[test]
    fn legacy_secret_variant_hashes_the_hash() {
        let secret = login_hash("admin", "admin");
        assert_eq!(
            realm_hash_with_legacy_secret("admin", "abcd1234", &secret),
            md5_hex(&format!("adminabcd1234{secret}"))
        );
    }
}
