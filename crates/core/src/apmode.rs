use std::fmt;
use std::net::Ipv4Addr;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

pub const AP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 10, 1);
pub const AP_DVRIP_PORT: u16 = 34567;
pub const AP_USER: &str = "admin";
pub const CONFIG_PORT: u16 = 8001;
pub const CONFIG_TARGET: Ipv4Addr = Ipv4Addr::BROADCAST;
pub const CONFIG_NAME: &str = "NetWork.Wifi";
pub const CONFIG_TIMEOUT_MS: u64 = 2000;
pub const CONNECT_TIMEOUT_MS: u64 = 3000;
pub const HEADER_WORD: u16 = 0x0514;
pub const BLOCK_MARK_WORD: u16 = 0x0515;
pub const BLOCK_MARK_AT_WORD: usize = 32;
pub const FIRST_SYMBOL: u16 = 0x80;
pub const SLOT_STEP: u16 = 8;
pub const GROUP_STEP: u16 = 64;
pub const MAX_WORDS: usize = 186;
pub const PAYLOAD_BYTES: usize = 2048;
pub const PAYLOAD_FILL: u8 = 0x65;
pub const PAYLOAD_FILL_BYTES: usize = 10;
pub const PAYLOAD_TEXT_OFFSET: usize = 10;
pub const PAYLOAD_TEXT_MAX: usize = 512;
pub const TRAILER_BYTES: usize = 500;
pub const SYMBOL_GAP: Duration = Duration::from_millis(20);
pub const ROUND_GAP: Duration = Duration::from_millis(5);
pub const TRAILER_GAP_MS: u64 = 51;
pub const SSID_MAX: usize = 34;
pub const PASSWORD_MAX: usize = 132;
const RECORD_OVERHEAD: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApError {
    SsidTooLong(usize),
    PasswordTooLong(usize),
    RecordTooLong(usize),
}

impl fmt::Display for ApError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApError::SsidTooLong(n) => write!(f, "ssid is {n} bytes, limit {SSID_MAX}"),
            ApError::PasswordTooLong(n) => write!(f, "password is {n} bytes, limit {PASSWORD_MAX}"),
            ApError::RecordTooLong(n) => write!(
                f,
                "record of {n} bytes needs more than {MAX_WORDS} symbols, which is the window the vendor sender reserves"
            ),
        }
    }
}

impl std::error::Error for ApError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credentials<'a> {
    pub ssid: &'a str,
    pub password: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub info: String,
    pub record: Vec<u8>,
    pub words: Vec<u16>,
    pub payload: Vec<u8>,
}

impl Credentials<'_> {
    pub fn info(&self, destination: Ipv4Addr) -> String {
        format!("S:{}P:{}ip:{destination}", self.ssid, self.password)
    }

    pub fn record(&self, destination: Ipv4Addr) -> Result<Vec<u8>, ApError> {
        if self.ssid.len() > SSID_MAX {
            return Err(ApError::SsidTooLong(self.ssid.len()));
        }
        if self.password.len() > PASSWORD_MAX {
            return Err(ApError::PasswordTooLong(self.password.len()));
        }
        let total = self.ssid.len() + self.password.len() + RECORD_OVERHEAD;
        let mut record = Vec::with_capacity(total);
        record.push(total as u8);
        record.push(b'A');
        record.push(self.ssid.len() as u8);
        record.push(self.password.len() as u8);
        record.extend_from_slice(self.ssid.as_bytes());
        record.extend_from_slice(self.password.as_bytes());
        record.push(destination.octets()[3]);
        let checksum: u16 = record.iter().map(|b| u16::from(*b)).sum();
        record.extend_from_slice(&checksum.to_le_bytes());
        Ok(record)
    }

    pub fn plan(&self, destination: Ipv4Addr) -> Result<Plan, ApError> {
        let record = self.record(destination)?;
        let words = symbols(&record)?;
        let info = self.info(destination);
        let payload = payload(&info);
        Ok(Plan {
            info,
            record,
            words,
            payload,
        })
    }
}

pub fn word_count(record_len: usize) -> usize {
    let groups = record_len / 3;
    let marks = usize::from(groups > BLOCK_MARK_AT_WORD / 8) * 3;
    let tail = match record_len % 3 {
        0 => 0,
        1 => 3,
        _ => 6,
    };
    3 + groups * 8 + marks + tail
}

pub fn symbols(record: &[u8]) -> Result<Vec<u16>, ApError> {
    let total = word_count(record.len());
    if total > MAX_WORDS {
        return Err(ApError::RecordTooLong(record.len()));
    }
    let mut words = Vec::with_capacity(total);
    words.extend_from_slice(&[HEADER_WORD; 3]);
    let groups = record.len() / 3;
    for group in 0..groups {
        if group * 8 == BLOCK_MARK_AT_WORD {
            words.extend_from_slice(&[BLOCK_MARK_WORD; 3]);
        }
        push_slots(&mut words, group, &record[group * 3..group * 3 + 3], 8);
    }
    let start = groups * 3;
    let tail = &record[start..];
    if !tail.is_empty() {
        let slots = if tail.len() == 1 { 3 } else { 6 };
        push_slots(&mut words, groups, tail, slots);
    }
    Ok(words)
}

fn push_slots(words: &mut Vec<u16>, group: usize, bytes: &[u8], slots: usize) {
    let mut value = [0u8; 4];
    value[..bytes.len()].copy_from_slice(bytes);
    let value = u32::from_le_bytes(value);
    let base = FIRST_SYMBOL + (group as u16) * GROUP_STEP;
    for slot in 0..slots {
        let bits = ((value >> (3 * slot)) & 0b111) as u16;
        words.push(base + (slot as u16) * SLOT_STEP + bits);
    }
}

pub fn payload(info: &str) -> Vec<u8> {
    let mut buffer = vec![0u8; PAYLOAD_BYTES];
    buffer[..PAYLOAD_FILL_BYTES].fill(PAYLOAD_FILL);
    let text = info.as_bytes();
    let length = text
        .len()
        .min(PAYLOAD_TEXT_MAX)
        .min(PAYLOAD_BYTES - PAYLOAD_TEXT_OFFSET);
    buffer[PAYLOAD_TEXT_OFFSET..PAYLOAD_TEXT_OFFSET + length].copy_from_slice(&text[..length]);
    buffer
}

pub fn datagram(payload: &[u8], length: usize) -> Option<&[u8]> {
    payload.get(..length)
}

pub fn trailer(payload: &[u8]) -> Option<&[u8]> {
    datagram(payload, TRAILER_BYTES)
}

pub fn wifi_config(existing: Option<&Value>, creds: &Credentials) -> Value {
    let mut wifi = match existing {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    };
    let secured = !creds.password.is_empty();
    wifi["Enable"] = json!(true);
    wifi["SSID"] = json!(creds.ssid);
    wifi["Keys"] = json!(creds.password);
    wifi["EncrypType"] = json!(if secured { "WPA2" } else { "OPEN" });
    wifi["Auth"] = json!(if secured { "AES" } else { "NONE" });
    wifi
}

pub fn wifi_block(config: &Value) -> Option<Value> {
    config
        .get("NetWork")
        .and_then(|section| section.get("Wifi"))
        .cloned()
        .filter(Value::is_object)
}

pub fn record_bytes(creds: &Credentials) -> usize {
    creds.ssid.len() + creds.password.len() + RECORD_OVERHEAD
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEST: Ipv4Addr = Ipv4Addr::BROADCAST;

    fn credentials() -> Credentials<'static> {
        Credentials {
            ssid: "CAM-LAB",
            password: "rahasia123",
        }
    }

    #[test]
    fn the_record_is_length_tag_sizes_credentials_last_octet_then_checksum() {
        let record = credentials().record(DEST).unwrap();
        assert_eq!(record[0] as usize, record.len());
        assert_eq!(record[1], b'A');
        assert_eq!(record[2] as usize, "CAM-LAB".len());
        assert_eq!(record[3] as usize, "rahasia123".len());
        assert_eq!(&record[4..11], b"CAM-LAB");
        assert_eq!(&record[11..21], b"rahasia123");
        assert_eq!(record[21], 255);
        let sum: u16 = record[..22].iter().map(|b| u16::from(*b)).sum();
        assert_eq!(&record[22..24], &sum.to_le_bytes());
        assert_eq!(record.len(), 24);
    }

    #[test]
    fn an_empty_password_marks_the_record_just_as_short() {
        let record = Credentials {
            ssid: "CAM",
            password: "",
        }
        .record(DEST)
        .unwrap();
        assert_eq!(record[3], 0);
        assert_eq!(record.len(), 10);
        let sum: u16 = record[..8].iter().map(|b| u16::from(*b)).sum();
        assert_eq!(&record[8..], &sum.to_le_bytes());
    }

    #[test]
    fn every_symbol_carries_its_own_position_like_concrete_package_does() {
        let words = symbols(&credentials().record(DEST).unwrap()).unwrap();
        assert_eq!(words.len(), 70);
        assert_eq!(&words[..3], &[HEADER_WORD; 3]);
        assert_eq!(
            &words[3..11],
            &[0x80, 0x8b, 0x94, 0x98, 0xa4, 0xae, 0xb1, 0xb8]
        );
        assert_eq!(&words[35..38], &[BLOCK_MARK_WORD; 3]);
        assert_eq!(words[69], 0x278);
        let base = FIRST_SYMBOL + 7 * GROUP_STEP + 7 * SLOT_STEP;
        assert!(words[69] >= base && words[69] <= base + 7);
    }

    #[test]
    fn a_tail_leaves_the_last_group_short_and_skips_the_marker() {
        let ten = Credentials {
            ssid: "ABC",
            password: "",
        }
        .record(DEST)
        .unwrap();
        assert_eq!(ten.len(), 10);
        let words = symbols(&ten).unwrap();
        assert_eq!(words.len(), 30);
        assert_eq!(word_count(ten.len()), 30);
        assert!(!words.contains(&BLOCK_MARK_WORD));

        let fourteen = Credentials {
            ssid: "ABCDEFG",
            password: "",
        }
        .record(DEST)
        .unwrap();
        assert_eq!(fourteen.len(), 14);
        let words = symbols(&fourteen).unwrap();
        assert_eq!(words.len(), 41);
        assert!(!words.contains(&BLOCK_MARK_WORD));

        let twenty_three = Credentials {
            ssid: "CAM-LAB-1",
            password: "rahasia",
        }
        .record(DEST)
        .unwrap();
        assert_eq!(twenty_three.len(), 23);
        let words = symbols(&twenty_three).unwrap();
        assert_eq!(words.len(), 68);
        assert_eq!(words.len(), word_count(23));
        assert_eq!(
            words
                .iter()
                .filter(|word| **word == BLOCK_MARK_WORD)
                .count(),
            3
        );
    }

    #[test]
    fn the_word_count_matches_the_binary_formula() {
        for length in [10usize, 14, 23, 24, 47] {
            let groups = length / 3;
            let marks = if groups > 4 { 3 } else { 0 };
            let tail = match length % 3 {
                0 => 0,
                1 => 3,
                _ => 6,
            };
            assert_eq!(word_count(length), 3 + groups * 8 + marks + tail);
            let words = symbols(&vec![b'x'; length]).unwrap();
            assert_eq!(words.len(), word_count(length));
        }
        assert_eq!(word_count(47), 132);
    }

    #[test]
    fn a_tail_of_one_byte_writes_three_slots_and_two_bytes_six() {
        let one = symbols(b"abcd").unwrap();
        assert_eq!(one.len(), 3 + 8 + 3);
        let two = symbols(b"abcde").unwrap();
        assert_eq!(two.len(), 3 + 8 + 6);
        let tail = &one[11..];
        let value = 0x64u32;
        for (slot, word) in tail.iter().enumerate() {
            let bits = ((value >> (3 * slot)) & 7) as u16;
            assert_eq!(
                *word,
                FIRST_SYMBOL + GROUP_STEP + (slot as u16) * SLOT_STEP + bits
            );
        }
    }

    #[test]
    fn the_payload_is_ten_e_bytes_then_the_info_string_then_zeros() {
        let plan = credentials().plan(DEST).unwrap();
        assert_eq!(plan.payload.len(), PAYLOAD_BYTES);
        assert_eq!(&plan.payload[..10], &[PAYLOAD_FILL; 10]);
        assert!(plan.payload[10..10 + plan.info.len()]
            .iter()
            .copied()
            .eq(plan.info.bytes()));
        assert!(plan.payload[10 + plan.info.len()..].iter().all(|b| *b == 0));
        assert!(plan
            .info
            .starts_with("S:CAM-LABP:rahasia123ip:255.255.255.255"));
    }

    #[test]
    fn a_datagram_is_the_payload_prefix_named_by_the_symbol() {
        let plan = credentials().plan(DEST).unwrap();
        for word in &plan.words {
            let slice = datagram(&plan.payload, usize::from(*word)).unwrap();
            assert_eq!(slice.len(), usize::from(*word));
            assert!(usize::from(*word) <= PAYLOAD_BYTES);
        }
        assert_eq!(trailer(&plan.payload).unwrap().len(), TRAILER_BYTES);
        assert!(datagram(&plan.payload, PAYLOAD_BYTES + 1).is_none());
    }

    #[test]
    fn oversized_credentials_and_records_are_refused_before_anything_is_sent() {
        let long_ssid = "s".repeat(SSID_MAX + 1);
        let error = Credentials {
            ssid: &long_ssid,
            password: "",
        }
        .record(DEST)
        .unwrap_err();
        assert_eq!(error, ApError::SsidTooLong(SSID_MAX + 1));
        let long_password = "p".repeat(PASSWORD_MAX + 1);
        let error = Credentials {
            ssid: "ok",
            password: &long_password,
        }
        .record(DEST)
        .unwrap_err();
        assert_eq!(error, ApError::PasswordTooLong(PASSWORD_MAX + 1));
        let wide = vec![b'x'; 100];
        assert!(matches!(symbols(&wide), Err(ApError::RecordTooLong(100))));
    }

    #[test]
    fn a_secured_network_sets_wpa2_aes_and_an_open_one_is_flagged_as_such() {
        let secured = wifi_config(None, &credentials());
        assert_eq!(secured["SSID"], "CAM-LAB");
        assert_eq!(secured["Keys"], "rahasia123");
        assert_eq!(secured["EncrypType"], "WPA2");
        assert_eq!(secured["Auth"], "AES");
        assert_eq!(secured["Enable"], true);
        let open = wifi_config(
            None,
            &Credentials {
                ssid: "CAM",
                password: "",
            },
        );
        assert_eq!(open["EncrypType"], "OPEN");
        assert_eq!(open["Auth"], "NONE");
    }

    #[test]
    fn the_device_block_is_merged_so_unknown_fields_survive() {
        let existing = json!({ "SSID": "old", "Channel": 6, "HostIP": "192.168.1.7" });
        let merged = wifi_config(Some(&existing), &credentials());
        assert_eq!(merged["Channel"], 6);
        assert_eq!(merged["HostIP"], "192.168.1.7");
        assert_eq!(merged["SSID"], "CAM-LAB");
        let wrapped = json!({ "NetWork": { "Wifi": merged.clone() } });
        assert_eq!(wifi_block(&wrapped).unwrap()["SSID"], "CAM-LAB");
        assert_eq!(record_bytes(&credentials()), 24);
    }
}
