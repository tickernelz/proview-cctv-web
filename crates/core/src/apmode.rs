use std::fmt;
use std::net::Ipv4Addr;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

pub const AP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 10, 1);
pub const AP_DVRIP_PORT: u16 = 34567;
pub const CONFIG_PORT: u16 = 8001;
pub const CONFIG_NAME: &str = "NetWork.Wifi";
pub const CONFIG_TIMEOUT_MS: u64 = 2000;
pub const PACKET_BYTES: usize = 1300;
pub const WINDOW_BYTES: usize = PAYLOAD_OFFSET + BLOCK_MARK_WORD as usize;
pub const HEADER_WORD: u16 = 0x0514;
pub const BLOCK_MARK_WORD: u16 = 0x0515;
pub const BLOCK_MARK_AT_WORD: usize = 32;
pub const FIRST_SYMBOL: u16 = 0x80;
pub const PAYLOAD_OFFSET: usize = 352;
pub const TRAILER_BYTES: usize = 500;
pub const SYMBOL_GAP: Duration = Duration::from_millis(1);
pub const ROUND_GAP: Duration = Duration::from_millis(100);
pub const ROUNDS: u32 = 50;
pub const SSID_MAX: usize = 34;
pub const PASSWORD_MAX: usize = 132;
const RECORD_OVERHEAD: usize = 7;
const SSID_START: usize = 4;

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
                "encoded record is {n} bytes, one packet carries {}",
                (PACKET_BYTES / 2 - 3) / 8 * 3
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
    pub packet: Vec<u8>,
    pub window: Vec<u8>,
    pub datagram_bytes: Vec<usize>,
    pub trailer_bytes: usize,
}

impl Credentials<'_> {
    fn check(&self) -> Result<(), ApError> {
        if self.ssid.len() > SSID_MAX {
            return Err(ApError::SsidTooLong(self.ssid.len()));
        }
        if self.password.len() > PASSWORD_MAX {
            return Err(ApError::PasswordTooLong(self.password.len()));
        }
        Ok(())
    }

    pub fn info(&self, destination: Ipv4Addr) -> String {
        format!("S:{}P:{}ip:{destination}", self.ssid, self.password)
    }

    pub fn record(&self, destination: Ipv4Addr) -> Result<Vec<u8>, ApError> {
        self.check()?;
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
        let packet = packet(&words);
        let datagram_bytes = words.iter().map(|w| usize::from(*w)).collect();
        let mut window = packet.to_vec();
        window.resize(WINDOW_BYTES, 0);
        Ok(Plan {
            info: self.info(destination),
            record,
            words,
            packet: packet.to_vec(),
            window,
            datagram_bytes,
            trailer_bytes: TRAILER_BYTES,
        })
    }
}

pub fn symbols(record: &[u8]) -> Result<Vec<u16>, ApError> {
    let groups = record.len().div_ceil(3);
    let marks = usize::from(groups > BLOCK_MARK_AT_WORD / 8) * 3;
    if 3 + groups * 8 + marks > PACKET_BYTES / 2 {
        return Err(ApError::RecordTooLong(record.len()));
    }
    let mut words = vec![HEADER_WORD; 3];
    for group in 0..groups {
        if group * 8 == BLOCK_MARK_AT_WORD {
            words.extend_from_slice(&[BLOCK_MARK_WORD; 3]);
        }
        let start = group * 3;
        let end = (start + 3).min(record.len());
        let mut chunk = [0u8; 4];
        chunk[..end - start].copy_from_slice(&record[start..end]);
        let value = u32::from_le_bytes(chunk);
        for slot in 0..8 {
            let bits = ((value >> (3 * slot)) & 0b111) as u16;
            words.push(FIRST_SYMBOL + (group as u16) * 8 + bits);
        }
    }
    Ok(words)
}

pub fn packet(words: &[u16]) -> [u8; PACKET_BYTES] {
    let mut packet = [0u8; PACKET_BYTES];
    for (index, word) in words.iter().take(PACKET_BYTES / 2).enumerate() {
        packet[index * 2..index * 2 + 2].copy_from_slice(&word.to_le_bytes());
    }
    packet
}

pub fn datagram(window: &[u8], length: usize) -> Option<&[u8]> {
    window.get(PAYLOAD_OFFSET..PAYLOAD_OFFSET + length)
}

pub fn trailer(window: &[u8]) -> Option<&[u8]> {
    datagram(window, TRAILER_BYTES)
}

pub fn decode(words: &[u16]) -> Option<Vec<u8>> {
    let body = words.get(3..)?;
    let mut record = Vec::new();
    let mut group_start = 0;
    let mut index = 0;
    while index < body.len() {
        if group_start * 8 == BLOCK_MARK_AT_WORD {
            let mark = body.get(index..index + 3)?;
            if mark.iter().any(|word| *word != BLOCK_MARK_WORD) {
                return None;
            }
            index += 3;
        }
        let slots = body.get(index..index + 8)?;
        let mut value = 0u32;
        for (slot, word) in slots.iter().enumerate() {
            let base = FIRST_SYMBOL + (group_start as u16) * 8;
            if *word < base || *word >= base + 8 {
                return None;
            }
            value |= u32::from(*word - base) << (3 * slot);
        }
        record.extend_from_slice(&value.to_le_bytes()[..3]);
        index += 8;
        group_start += 1;
    }
    Some(record)
}

pub fn record_extent(record: &[u8]) -> Option<usize> {
    record.first().map(|length| usize::from(*length))
}

pub fn ssid_of(record: &[u8]) -> Option<&[u8]> {
    let total = record_extent(record)?;
    let length = usize::from(*record.get(2)?);
    if total < length + RECORD_OVERHEAD {
        return None;
    }
    record.get(SSID_START..SSID_START + length)
}

pub fn password_of(record: &[u8]) -> Option<&[u8]> {
    let ssid = ssid_of(record)?;
    let length = usize::from(*record.get(3)?);
    let start = SSID_START + ssid.len();
    record.get(start..start + length)
}

pub fn checksum_ok(record: &[u8]) -> bool {
    let body = match record.len().checked_sub(2) {
        Some(body) => body,
        None => return false,
    };
    let expected: u16 = record[..body].iter().map(|b| u16::from(*b)).sum();
    record[body..] == expected.to_le_bytes()
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

pub fn wifi_body(creds: &Credentials, existing: Option<&Value>) -> Value {
    json!({ "Wifi": wifi_config(existing, creds) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEST: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 255);

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
        assert_eq!(record.len(), 7 + 7 + 10);
        assert!(checksum_ok(&record));
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
        assert_eq!(record.len(), 7 + 3);
        assert!(checksum_ok(&record));
    }

    #[test]
    fn every_three_bytes_become_eight_words_on_one_base() {
        let record = credentials().record(DEST).unwrap();
        let words = symbols(&record).unwrap();
        assert_eq!(&words[..3], &[HEADER_WORD; 3]);
        assert_eq!(&words[35..38], &[BLOCK_MARK_WORD; 3]);
        let groups = record.len().div_ceil(3);
        assert_eq!(words.len(), 3 + groups * 8 + 3);
        let first = u32::from_le_bytes([record[0], record[1], record[2], 0]);
        for slot in 0..8 {
            let bits = ((first >> (3 * slot)) & 7) as u16;
            assert_eq!(words[3 + slot], FIRST_SYMBOL + bits);
        }
        let base = FIRST_SYMBOL + ((groups - 1) as u16) * 8;
        let last = words.len() - 8;
        for slot in 0..8 {
            let word = words[last + slot];
            assert!(
                word >= base && word < base + 8,
                "{word:#x} outside {base:#x}"
            );
        }
    }

    #[test]
    fn the_encoder_round_trips_the_record() {
        let record = credentials().record(DEST).unwrap();
        let words = symbols(&record).unwrap();
        let decoded = decode(&words).unwrap();
        assert_eq!(&decoded[..record.len()], &record[..]);
        assert_eq!(ssid_of(&decoded[..record.len()]).unwrap(), b"CAM-LAB");
        assert_eq!(
            password_of(&decoded[..record.len()]).unwrap(),
            b"rahasia123"
        );
    }

    #[test]
    fn the_packet_carries_the_words_little_endian_and_stays_1300_bytes() {
        let record = credentials().record(DEST).unwrap();
        let words = symbols(&record).unwrap();
        let packet = packet(&words);
        assert_eq!(packet.len(), PACKET_BYTES);
        assert_eq!(&packet[..2], &HEADER_WORD.to_le_bytes());
        assert_eq!(&packet[6..8], &words[3].to_le_bytes());
        assert!(packet[words.len() * 2..].iter().all(|b| *b == 0));
    }

    #[test]
    fn a_datagram_is_the_packet_slice_named_by_the_symbol() {
        let plan = credentials().plan(DEST).unwrap();
        assert_eq!(plan.datagram_bytes.len(), plan.words.len());
        for (index, length) in plan.datagram_bytes.iter().enumerate() {
            assert_eq!(*length, usize::from(plan.words[index]));
            assert!(
                (0x80usize..0x100).contains(length)
                    || *length == PACKET_BYTES
                    || *length == BLOCK_MARK_WORD as usize,
                "{length} is neither a data symbol nor a marker"
            );
            assert!(datagram(&plan.window, *length).is_some());
        }
        assert_eq!(
            plan.datagram_bytes
                .iter()
                .filter(|length| **length == BLOCK_MARK_WORD as usize)
                .count(),
            3
        );
        assert!(datagram(&plan.window, BLOCK_MARK_WORD as usize).is_some());
        assert_eq!(plan.window.len(), WINDOW_BYTES);
        assert!(trailer(&plan.window).is_some());
    }

    #[test]
    fn oversized_credentials_are_refused_before_anything_is_sent() {
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
        let body = wifi_body(&credentials(), Some(&existing));
        assert_eq!(body["Wifi"]["SSID"], "CAM-LAB");
        let wrapped = json!({ "NetWork": { "Wifi": body["Wifi"].clone() } });
        assert_eq!(wifi_block(&wrapped).unwrap()["SSID"], "CAM-LAB");
    }
}
