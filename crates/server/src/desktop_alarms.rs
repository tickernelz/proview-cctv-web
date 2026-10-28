use std::sync::Mutex;
use std::time::{Duration, Instant};

use proview_core::cloud::alarm::PAGE_LIMIT;
use proview_core::cloud::{AlarmClient, AlarmEntry, CloudDevice, EnvelopeClient, Identity};
use proview_core::session::Store;
use serde::Serialize;

use crate::routes::alarms::{now_seconds, page_for, stamp_at, AlarmQueryError};

const SLACK_SECS: i64 = 14 * 3600;
const DEVICE_CACHE_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopAlarm {
    pub id: String,
    pub serial: String,
    pub channel: String,
    pub device: String,
    pub event: String,
    pub time: String,
    pub utc_time: i64,
}

impl DesktopAlarm {
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.serial, self.channel, self.id)
    }
}

pub struct DesktopAlarms {
    envelope: EnvelopeClient,
    alarms: AlarmClient,
    devices: Mutex<Option<(Instant, String, Vec<CloudDevice>)>>,
}

impl DesktopAlarms {
    pub fn from_env() -> Self {
        Self {
            envelope: EnvelopeClient::new(Identity::resolve()),
            alarms: AlarmClient::from_env(),
            devices: Mutex::new(None),
        }
    }

    fn cached_devices(&self, account: &str) -> Option<Vec<CloudDevice>> {
        let guard = self.devices.lock().ok()?;
        let (seen, owner, devices) = guard.as_ref()?;
        (owner == account && seen.elapsed() < DEVICE_CACHE_TTL).then(|| devices.clone())
    }

    fn keep_devices(&self, account: &str, devices: &[CloudDevice]) {
        if let Ok(mut guard) = self.devices.lock() {
            *guard = Some((Instant::now(), account.to_string(), devices.to_vec()));
        }
    }

    pub async fn since(&self, sessions: &Store, from: i64) -> Result<Vec<DesktopAlarm>, String> {
        let (_, session) = sessions
            .latest()
            .ok_or_else(|| "not signed in".to_string())?;
        let token = session
            .token
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                "alarms read the vendor cloud; sign in with the cloud account".to_string()
            })?;
        let devices = match self.cached_devices(&session.account) {
            Some(devices) => devices,
            None => {
                let devices = self
                    .envelope
                    .device_list(&token)
                    .await
                    .map_err(|error| error.to_string())?;
                self.keep_devices(&session.account, &devices);
                devices
            }
        };
        let (start, end) = window(from, now_seconds());
        let mut entries: Vec<DesktopAlarm> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        for device in devices {
            match page_for(&self.alarms, &device, &start, &end, PAGE_LIMIT).await {
                Ok(page) => entries.extend(collect(&device, &page.entries)),
                Err(AlarmQueryError::NoAuthCode) => {
                    tracing::debug!(serial = %device.uuid, "no cloud auth code, skipped");
                }
                Err(AlarmQueryError::Cloud(error)) => {
                    tracing::warn!(serial = %device.uuid, %error, "alarm centre call failed");
                    failures.push(error.to_string());
                }
            }
        }
        if entries.is_empty() && !failures.is_empty() {
            return Err(failures.remove(0));
        }
        entries.sort_by(|a, b| {
            a.utc_time
                .cmp(&b.utc_time)
                .then_with(|| a.key().cmp(&b.key()))
        });
        entries.dedup_by(|a, b| a.key() == b.key());
        Ok(entries)
    }
}

pub fn window(from: i64, now: i64) -> (String, String) {
    (stamp_at(from - SLACK_SECS), stamp_at(now + SLACK_SECS))
}

pub fn collect(device: &CloudDevice, entries: &[AlarmEntry]) -> Vec<DesktopAlarm> {
    let name = device.display_name();
    entries
        .iter()
        .map(|entry| DesktopAlarm {
            id: entry.id.clone(),
            serial: device.uuid.clone(),
            channel: entry.channel.trim().to_string(),
            device: if entry.device_name.trim().is_empty() {
                name.to_string()
            } else {
                entry.device_name.trim().to_string()
            },
            event: if entry.event_name.trim().is_empty() {
                entry.event.clone()
            } else {
                entry.event_name.trim().to_string()
            },
            time: entry.time.clone(),
            utc_time: entry.utc_time,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(uuid: &str, nickname: &str) -> CloudDevice {
        serde_json::from_value(serde_json::json!({
            "uuid": uuid,
            "nickname": nickname,
        }))
        .expect("device")
    }

    fn entry(id: &str, channel: &str, utc_time: i64) -> AlarmEntry {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "event": "motion",
            "eventName": "Motion",
            "time": "2026-10-28 01:02:03",
            "utcTime": utc_time,
            "channel": channel,
        }))
        .expect("entry")
    }

    #[test]
    fn an_entry_carries_the_device_name_and_the_window_stamp() {
        let alarms = collect(&device("DEV0001", "Gate"), &[entry("a1", "0", 10)]);
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].device, "Gate");
        assert_eq!(alarms[0].serial, "DEV0001");
        assert_eq!(alarms[0].channel, "0");
        assert_eq!(alarms[0].event, "Motion");
        assert_eq!(alarms[0].time, "2026-10-28 01:02:03");
    }

    #[test]
    fn the_key_separates_two_devices_that_share_an_alarm_id() {
        let gate = collect(
            &device("DEV0001", "Gate"),
            &[entry("261027171005", "0", 10)],
        );
        let yard = collect(
            &device("DEV0002", "Yard"),
            &[entry("261027171005", "0", 10)],
        );
        assert_ne!(gate[0].key(), yard[0].key());
        assert_eq!(gate[0].id, yard[0].id);
    }

    #[test]
    fn the_key_separates_two_channels_that_share_an_alarm_id() {
        let one = collect(
            &device("DEV0001", "Gate"),
            &[entry("261027171005", "0", 10)],
        );
        let two = collect(
            &device("DEV0001", "Gate"),
            &[entry("261027171005", "1", 10)],
        );
        assert_ne!(one[0].key(), two[0].key());
    }

    #[test]
    fn a_stamp_is_utc_seconds_written_the_way_the_centre_wants() {
        assert_eq!(stamp_at(0), "1970-01-01 00:00:00");
        assert_eq!(stamp_at(1_600_000_000), "2020-09-13 12:26:40");
    }

    #[test]
    fn the_window_is_padded_by_fourteen_hours_on_each_side() {
        let (start, end) = window(1_600_000_000, 1_600_003_600);
        assert_eq!(
            start, "2020-09-12 22:26:40",
            "fourteen hours before the last alarm"
        );
        assert_eq!(end, "2020-09-14 03:26:40", "fourteen hours after the check");
    }

    #[tokio::test]
    async fn without_a_session_the_shell_is_told_to_sign_in() {
        let alarms = DesktopAlarms::from_env();
        let sessions = Store::new();
        let error = alarms
            .since(&sessions, now_seconds() - 60)
            .await
            .expect_err("no session");
        assert_eq!(error, "not signed in");
    }
}
