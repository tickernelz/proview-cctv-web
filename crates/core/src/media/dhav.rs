use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use base64::Engine as _;

pub const MEDIA_CHANNEL_KEY: [u8; 16] = *b"dashoiahfarqdasr";

const BLOCK: usize = 16;

pub struct MediaCipher {
    cipher: Aes128,
}

impl MediaCipher {
    pub fn new(key: &[u8; BLOCK]) -> Self {
        Self {
            cipher: Aes128::new(key.into()),
        }
    }

    pub fn channel() -> Self {
        Self::new(&MEDIA_CHANNEL_KEY)
    }

    pub fn encrypt(&self, plain: &[u8]) -> Vec<u8> {
        let padded = plain.len().div_ceil(BLOCK) * BLOCK;
        let mut out = vec![0u8; padded];
        out[..plain.len()].copy_from_slice(plain);
        let mut chain = [0u8; BLOCK];
        for chunk in out.chunks_mut(BLOCK) {
            for i in 0..BLOCK {
                chunk[i] ^= chain[i];
            }
            self.cipher.encrypt_block(chunk.into());
            chain.copy_from_slice(chunk);
        }
        out
    }

    pub fn decrypt(&self, cipher: &[u8]) -> Option<Vec<u8>> {
        if cipher.len() % BLOCK != 0 {
            return None;
        }
        let mut out = vec![0u8; cipher.len()];
        let mut chain = [0u8; BLOCK];
        for (at, chunk) in cipher.chunks(BLOCK).enumerate() {
            let mut block = [0u8; BLOCK];
            block.copy_from_slice(chunk);
            self.cipher.decrypt_block((&mut block).into());
            for i in 0..BLOCK {
                out[at * BLOCK + i] = block[i] ^ chain[i];
            }
            chain.copy_from_slice(chunk);
        }
        Some(out)
    }

    pub fn encrypt_body(&self, plain: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.encrypt(plain))
    }

    pub fn decrypt_body(&self, body: &[u8]) -> Option<Vec<u8>> {
        let text = trim_nul(body);
        let raw = base64::engine::general_purpose::STANDARD
            .decode(text)
            .ok()?;
        self.decrypt(&raw)
    }
}

pub fn trim_nul(body: &[u8]) -> &[u8] {
    let mut end = body.len();
    while end > 0 && body[end - 1] == 0 {
        end -= 1;
    }
    &body[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN_BODY: &str = "rfIQlMaR6Ses2pRV7x9RclY2y+81ry81l5Nec9HJvp+tSYe34Zar6IV+cWngFwDJDEEnKBpAvBsQvuuUabsRQswfaJUsou5eYdJAY29nU7CgTiNbpoKdll710wzVmt4yApKFqoLEsiGxBgUbhxUhdkmpx6eGTPgGu6DammRvDREMLwL8i/monuv9ejj9UfnAhNaismxvi985cY8HKkOXir+UTiL5l1iPhVMZw764dgZG+nEqCN3tPoTebOFxrOsoij73NRH5j3w9TKU/etg7/PREXsJr6zqz7GyvmK8W2/YyUXGxoJuy+XfIzzdOdCi61Wt2FXAmpSHbRCZZr96jYPWS9ZEEg7v44u2jczjOoiS2d1w3eh213CPwbRw106ofTIgRdVBPqoEgeg0LygYBbz5b5Lpg4aNV2O6BZjHJ+fdJqM07HRvVgxFuSGZgI4TOJ74bAYAp187wnAIxDfBchNSlehJpQ7rCP/xwbpmtx4M9/ahRBvQQdnIeEUTJf9fF";

    const LOGIN_TEXT: &str = r#"{"EncryptType":"TOKEN","LoginType":"DVRIP-FutureHome","UserName":"F0+7ghjHWzNsktOE","PassWord":"","CommunicateKey":"8B91764818E8ED2BC5BC63F9736A6B7CED20676FA2C10AB03B1942D30695906AD631D158C9792CF2AF9AFF847BB9F07D405E89BD4C05B7FAC3CFDECAA68DFF9FDBD20000FAF2138880DCAE0E8BF20D441617D2DDE4856ACAA697DA61917943F425350EB4C821BC7FF41826EC95E902B0A6C27D9079761097608A3D67D8E8E3AB"}"#;

    const CLAIM_BODY: &str = "7b39TFcOQLvicIPJ+ruy/zRNtJmETeNos+id7XiwLi9t7vW74uHQ3/gowFvs3VcOPAPBcQ5SI3wjU0jsgOwZaRrOgy+30/SwksjZY83qW8A6g75v6VTHpj+WETIGgyoY21TJEj8YR8fFl86ZdzkRrVd2jKZpx/HEOtX38igJTuE1WTDn1ZqTt7rKRmXRTtYSxkOrnNkjYmkb77il01KqkyPR0T9QyqdF3B1mipZ7XIktJqKwIJSBEG4L77OuiT48iGH8Yc+vc+DXOW+CABxp4dzcN8RoA4dlOu5ZKlSoEZzE4GIevtmNKm0Q4yFBaSS8nXPnWNcT95Q1548WGHtQ0YHNTzKbzmSTVNowjMirLUW5v/CASVvar2OEoGMEzj4uL6qQ+0incZSGu9DC+2g0Y/FrrT/SANyiFv9gmy8C+iLj9clqmBuFWAv7I7Zy+9KFaRcUg8lyoQM+xR0+exaU+W22xUeBkMPLHJ018tsfttQi3ZbfPStdRa5L5G9JjyOCqqrHmod5ocyHJj1T+F+kVqGU5fbGG4jbHb5+202spXo4xIZvOQQ5RqBLW5tkO5fEuWnuunxPfG/MLDdZzME+8GDN52G353wa3hM+KdBDuvS1BQ16J17wMxYDexdgXxgQcDMD9YOKBjLX0PM4YfQa5jswAV+CmC85K4CnjDkHukFKf46cuqJhs+8/gQbQ8/22dncYQU1EKPlSDKfizB7I0uqKuiUw0nEMQ5VHDsokDohkKXnNrfCO9aAt9cxD5cmSm77VWCZI6/Asz6EmZpmb8XpYwO3+RYFZddCFls483zyRk84CZP+IBtsWZZzrD2pS7gg/Ej411HxlUUcDZJ418r8hciB0XOeKZNQlm4l3csQFVYqafD4fxLUf26+2hqU3sp8ICI4iY6rmBUPToMewXkTG8eZskzY9fn+bS8/EyxpDk+s4vi19hpXD1Dk349N/";

    const CLAIM_HEAD: &str = r#"{ "Bits" : 1024, "DataEncryptionType" : { "AES" : true, "AESV2" : true, "VEKEY1" : true }, "EncryptAlgo" : "RSA_V1.5""#;

    const APP_LOGIN_BODY: &str = "rfIQlMaR6Ses2pRV7x9RclY2y+81ry81l5Nec9HJvp+tSYe34Zar6IV+cWngFwDJDEEnKBpAvBsQvuuUabsRQswfaJUsou5eYdJAY29nU7CgTiNbpoKdll710wzVmt4yApKFqoLEsiGxBgUbhxUhdnX8+yGUAiombRkLNY8pSACDbhJU39hPCxz1Lanf2o5sTwLXrDVEPfwjE3RFQXN1eSsBEQ7zDLOwZkUqLTAyLJmYcgIo2zNf2G1fS/3vmXj6etStSBQQyEnxTpgBFveAK04VqTx8NW2sl9N/ACf5S9h7s+9Sev4u7UimEIDUcg4WBPs3fTn2QaBbXiUd5xRAu9fFFzZR+omKtKf4niKUXvu6fY0qY811yD4AHMLfHWVa1pclBcNlxYMIDEB6VPVx5pq4A9FAKgGNljEp4E+BAPvqDO94+QapyzaWQ70lA/ZVMjUcGafq/C1lYsWGDSgva9QcYRBI6+Jn2o0CPm1S7z/Al8UGnFJ4tQws/Cq3I2iTP41NZ3OTzS/1RBGvbmlotpeH869p+0Ind61cmOE4O6L01aHAlzd2d+U86JoJPp1Z";

    const APP_LOGIN_TEXT: &str = r#"{"EncryptType":"TOKEN","LoginType":"DVRIP-FutureHome","UserName":"F0+7ghjHWzNsktOE","PassWord":"","CommunicateKey":"0AFEDF8BC5D5B7399A54B4427841E63FEF08910E6479B12DFA875C546027DFD84EFEFA53D624800B55055EAA14A3A0B679AE7DD3DE937129CC2DB91DC8272CFF39A4A5B42A58E911B2286CB516C8255EB8AD4F1408F47218AE71E68B5E9C882EB13D4E9162953E4A9605E09969178747E5537011E983B056B16D447C754B8B96","User-Agent":"Funv1.2.2c-And-wR5qH796Ky8D03r"}"#;

    const APP_CLAIM_BODY: &str = "7b39TFcOQLvicIPJ+ruy/zRNtJmETeNos+id7XiwLi9t7vW74uHQ3/gowFvs3VcOPAPBcQ5SI3wjU0jsgOwZaRrOgy+30/SwksjZY83qW8A6g75v6VTHpj+WETIGgyoY21TJEj8YR8fFl86ZdzkRrVd2jKZpx/HEOtX38igJTuE1WTDn1ZqTt7rKRmXRTtYSxkOrnNkjYmkb77il01KqkyPR0T9QyqdF3B1mipZ7XIktJqKwIJSBEG4L77OuiT48iGH8Yc+vc+DXOW+CABxp4dzcN8RoA4dlOu5ZKlSoEZzE4GIevtmNKm0Q4yFBaSS8nXPnWNcT95Q1548WGHtQ0YHNTzKbzmSTVNowjMirLUW5v/CASVvar2OEoGMEzj4uL6qQ+0incZSGu9DC+2g0Y/FrrT/SANyiFv9gmy8C+iLj9clqmBuFWAv7I7Zy+9KFaRcUg8lyoQM+xR0+exaU+W22xUeBkMPLHJ018tsfttQi3ZbfPStdRa5L5G9JjyOCqqrHmod5ocyHJj1T+F+kVqGU5fbGG4jbHb5+202spXo4xIZvOQQ5RqBLW5tkO5fEuWnuunxPfG/MLDdZzME+8GDN52G353wa3hM+KdBDuvS1BQ16J17wMxYDexdgXxgQcDMD9YOKBjLX0PM4YfQa5jswAV+CmC85K4CnjDkHukFKf46cuqJhs+8/gQbQ8/22dncYQU1EKPlSDKfizB7I0uqKuiUw0nEMQ5VHDsokDohkKXnNrfCO9aAt9cxD5cmSm77VWCZI6/Asz6EmZpmb8XpYwO3+RYFZddCFls483zyRk84CZP+IBtsWZZzrD2pS7gg/Ej411HxlUUcDZJ418r8hciB0XOeKZNQlm4l3csQFVYqafD4fxLUf26+2hqU3sp8ICI4iY6rmBUPToMewXkTG8eZskzY9fn+bS8/EyxpDk+s4vi19hpXD1Dk349N/";

    fn cipher() -> MediaCipher {
        MediaCipher::channel()
    }

    #[test]
    fn decrypts_the_captured_media_login() {
        let plain = cipher()
            .decrypt_body(LOGIN_BODY.as_bytes())
            .expect("login body");
        assert_eq!(plain.len(), 384);
        assert_eq!(trim_nul(&plain), LOGIN_TEXT.as_bytes());
    }

    #[test]
    fn decrypts_a_login_captured_from_the_vendor_app() {
        let plain = cipher()
            .decrypt_body(APP_LOGIN_BODY.as_bytes())
            .expect("app login body");
        assert_eq!(plain.len(), 432);
        assert_eq!(trim_nul(&plain), APP_LOGIN_TEXT.as_bytes());
        assert_eq!(APP_CLAIM_BODY, CLAIM_BODY);
    }

    #[test]
    fn reencrypts_the_media_login_byte_for_byte() {
        let mut plain = LOGIN_TEXT.as_bytes().to_vec();
        plain.push(0);
        assert_eq!(cipher().encrypt_body(&plain), LOGIN_BODY);
    }

    #[test]
    fn decrypts_the_captured_claim_reply() {
        let plain = cipher()
            .decrypt_body(CLAIM_BODY.as_bytes())
            .expect("claim body");
        assert_eq!(plain.len(), 720);
        let text = String::from_utf8_lossy(trim_nul(&plain)).to_string();
        assert!(text.starts_with(CLAIM_HEAD), "{text}");
        assert!(text.contains("\"PublicKey\" : \"91670C27"));
        assert!(text.contains("\"NotEncryptMsgID\""));
        assert!(text.ends_with("\"Ret\" : 100 }\n"));
    }

    #[test]
    fn rejects_bodies_that_are_not_block_aligned() {
        assert!(cipher().decrypt(&[0u8; 15]).is_none());
        assert!(cipher().decrypt(&[0u8; 17]).is_none());
        assert!(cipher().decrypt_body(b"not base64").is_none());
    }

    #[test]
    fn keeps_the_empty_payload_symmetric() {
        assert_eq!(cipher().encrypt(b""), Vec::<u8>::new());
        assert_eq!(cipher().decrypt(&[]), Some(Vec::new()));
        assert_eq!(cipher().encrypt_body(b""), "");
        assert_eq!(cipher().decrypt_body(b""), Some(Vec::new()));
    }

    #[test]
    fn pads_short_payloads_to_one_block() {
        let sealed = cipher().encrypt(b"abc");
        assert_eq!(sealed.len(), 16);
        assert_eq!(cipher().decrypt(&sealed).unwrap()[..3], *b"abc");
    }

    #[test]
    fn trims_the_zero_terminator_only() {
        assert_eq!(trim_nul(b"json\0\0"), b"json");
        assert_eq!(trim_nul(b"json"), b"json");
        assert_eq!(trim_nul(b"\0\0"), b"");
    }
}
