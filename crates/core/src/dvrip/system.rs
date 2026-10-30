use serde::Serialize;
use serde_json::{json, Map, Value};

use super::client::{require_ok, Connection, DvrError};
use super::crypto::sofia_hash;
use super::msg;

pub const DEFAULT_SECTIONS: [&str; 10] = [
    "Account",
    "Alarm",
    "CameraPARAM",
    "CommPtz",
    "Encode",
    "General",
    "NetCommon",
    "NetServer",
    "Preview",
    "Record",
];

pub const PING_HOSTS: [&str; 4] = ["223.5.5.5", "8.8.8.8", "secu100.net", "www.microsoft.com"];

const UPGRADE_CHUNK: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceUser {
    pub name: String,
    pub group: Option<String>,
    pub memo: Option<String>,
    pub reserved: Option<bool>,
    pub sharable: Option<bool>,
    pub authorities: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PingReport {
    pub url: String,
    pub reachable: bool,
    pub average_ms: Option<i64>,
    pub lost_percent: Option<i64>,
    pub sent: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceTime {
    pub device_time: Option<String>,
    pub time_zone: Value,
    pub location: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebootAck {
    Answered(i64),
    LinkClosed,
}

pub fn session_field(session: u32) -> String {
    format!("0x{session:08X}")
}

pub fn reboot_body(session: u32) -> Value {
    json!({
        "Name": "OPMachine",
        "OPMachine": { "Action": "Reboot" },
        "SessionID": session_field(session),
    })
}

pub fn default_config_body(session: u32) -> Value {
    let mut sections = Map::new();
    for name in DEFAULT_SECTIONS {
        sections.insert(name.to_string(), json!(1));
    }
    json!({
        "Name": "OPDefaultConfig",
        "OPDefaultConfig": Value::Object(sections),
        "SessionID": session_field(session),
    })
}

pub fn time_setting_body(session: u32, time: &str) -> Value {
    json!({
        "Name": "OPTimeSetting",
        "OPTimeSetting": time,
        "SessionID": session_field(session),
    })
}

pub fn time_zone_value(minutes_east_of_utc: i64) -> Value {
    json!({ "FirstUserTimeZone": 0, "timeMin": -minutes_east_of_utc })
}

pub fn minutes_east_of_utc(time_zone: &Value) -> Option<i64> {
    time_zone
        .get("timeMin")
        .and_then(Value::as_i64)
        .map(|minutes| -minutes)
}

pub fn valid_device_time(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| match index {
        4 | 7 => *byte == b'-',
        10 => *byte == b' ',
        13 | 16 => *byte == b':',
        _ => byte.is_ascii_digit(),
    })
}

pub fn ping_body(session: u32, url: &str, count: i64, timeout_secs: i64) -> Value {
    json!({
        "Name": "Ping",
        "Ping": { "Num": count, "Timeout": timeout_secs, "URL": url },
        "SessionID": session_field(session),
    })
}

pub fn modify_password_body(session: u32, user: &str, old: &str, new: &str) -> Value {
    json!({
        "EncryptType": "MD5",
        "NewPassWord": sofia_hash(new),
        "PassWord": sofia_hash(old),
        "SessionID": session_field(session),
        "UserName": user,
    })
}

pub fn modify_user_body(session: u32, current: &str, user: &Value, new_name: &str) -> Value {
    let mut renamed = user.clone();
    renamed["Name"] = json!(new_name);
    json!({
        "Name": "ModifyUser",
        "UserName": current,
        "User": renamed,
        "SessionID": session_field(session),
    })
}

pub fn safety_body(session: u32, name: &str, inner: Option<Value>) -> Value {
    let mut body = json!({ "Name": name, "SessionId": session_field(session) });
    if let Some(inner) = inner {
        body[name] = inner;
    }
    body
}

pub fn safety_answer_value(answers: &[String]) -> Value {
    json!({ "Answer": answers })
}

pub fn new_password_value(user: &str, password: &str) -> Value {
    json!({ "UserName": user, "NewPassword": password })
}

pub fn version_request_body(session: u32, language: &str) -> Value {
    json!({
        "Name": "OPVersionReq",
        "OPVersionReq": { "Expect": 0, "Language": language, "Manual": 1 },
        "SessionID": session_field(session),
    })
}

pub fn start_cloud_upgrade_body(
    session: u32,
    serial: &str,
    dev_id: &str,
    file_name: &str,
    version_date: &str,
) -> Value {
    json!({
        "Name": "OPStartUpgradeReq",
        "OPStartUpgradeReq": {
            "UUID": serial,
            "DevID": dev_id,
            "FileName": file_name,
            "VerDate": version_date,
            "Manual": 1,
        },
        "SessionID": session_field(session),
    })
}

pub fn file_upgrade_body(session: u32, action: &str, size: usize) -> Value {
    let mut inner = json!({ "Action": action, "Type": "System" });
    if action == "Start" {
        inner["FileSize"] = json!(size);
    }
    json!({
        "Name": "OPSystemUpgrade",
        "OPSystemUpgrade": inner,
        "SessionID": session_field(session),
    })
}

pub fn parse_users(reply: &Value) -> Vec<DeviceUser> {
    reply
        .get("Users")
        .and_then(Value::as_array)
        .map(|users| {
            users
                .iter()
                .filter_map(|user| {
                    Some(DeviceUser {
                        name: user.get("Name")?.as_str()?.to_string(),
                        group: user
                            .get("Group")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        memo: user.get("Memo").and_then(Value::as_str).map(str::to_string),
                        reserved: user.get("Reserved").and_then(Value::as_bool),
                        sharable: user.get("Sharable").and_then(Value::as_bool),
                        authorities: user
                            .get("AuthorityList")
                            .and_then(Value::as_array)
                            .map(Vec::len)
                            .unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_ping(url: &str, count: i64, reply: &Value) -> PingReport {
    let inner = reply.get("Ping").cloned().unwrap_or(Value::Null);
    PingReport {
        url: url.to_string(),
        reachable: inner
            .get("Result")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        average_ms: inner.get("Avdelay").and_then(Value::as_i64),
        lost_percent: inner.get("Lost").and_then(Value::as_i64),
        sent: count,
    }
}

pub fn ability_flags(abilities: &Value) -> Vec<String> {
    let mut flags = Vec::new();
    if let Some(groups) = abilities.as_object() {
        for (group, entries) in groups {
            if let Some(entries) = entries.as_object() {
                for (name, value) in entries {
                    if value.as_bool() == Some(true) {
                        flags.push(format!("{group}/{name}"));
                    }
                }
            }
        }
    }
    flags.sort();
    flags
}

pub fn ability(abilities: &Value, path: &str) -> bool {
    let mut node = abilities;
    for part in path.split('/') {
        match node.get(part) {
            Some(next) => node = next,
            None => return false,
        }
    }
    node.as_bool().unwrap_or(false) || node.as_i64().map(|value| value > 0).unwrap_or(false)
}

pub fn firmware_dev_id(software_version: &str) -> Option<String> {
    let parts: Vec<&str> = software_version.trim().split('.').collect();
    if parts.len() <= 5 {
        return None;
    }
    Some(parts[3..].concat())
}

pub fn firmware_build_date(build_time: &str) -> Option<String> {
    let date = build_time.trim().get(..10)?;
    valid_device_time(&format!("{date} 00:00:00")).then(|| date.to_string())
}

pub fn hex_ipv4(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))?;
    let value = u32::from_str_radix(hex, 16).ok()?;
    let [a, b, c, d] = value.to_le_bytes();
    Some(format!("{a}.{b}.{c}.{d}"))
}

pub fn uptime_minutes(raw: &Value) -> Option<u64> {
    match raw {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => {
            let text = text.trim();
            match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                Some(hex) => u64::from_str_radix(hex, 16).ok(),
                None => text.parse().ok(),
            }
        }
        _ => None,
    }
}

impl Connection {
    async fn named(&mut self, id: u16, name: &str) -> Result<Value, DvrError> {
        let body = json!({ "Name": name, "SessionID": session_field(self.session()) });
        let reply = self.request(id, body).await?;
        require_ok(&reply)?;
        Ok(reply)
    }

    pub async fn system_entry(&mut self, name: &str) -> Result<Value, DvrError> {
        let reply = self.named(msg::SYSTEM_INFO, name).await?;
        Ok(reply.get(name).cloned().unwrap_or(Value::Null))
    }

    pub async fn ability_entry(&mut self, name: &str) -> Result<Value, DvrError> {
        let reply = self.named(msg::ABILITY, name).await?;
        Ok(reply.get(name).cloned().unwrap_or(Value::Null))
    }

    pub async fn time_query(&mut self) -> Result<Option<String>, DvrError> {
        let reply = self.named(msg::TIME_QUERY, "OPTimeQuery").await?;
        Ok(reply
            .get("OPTimeQuery")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    pub async fn set_time(&mut self, time: &str) -> Result<i64, DvrError> {
        let reply = self
            .request(msg::OP_MACHINE, time_setting_body(self.session(), time))
            .await?;
        require_ok(&reply)
    }

    pub async fn reboot(&mut self) -> Result<RebootAck, DvrError> {
        self.send_json(msg::OP_MACHINE, &reboot_body(self.session()))
            .await?;
        match self.await_reply(8).await {
            Ok(reply) => require_ok(&reply).map(RebootAck::Answered),
            Err(DvrError::Io(_)) | Err(DvrError::Frame(_)) | Err(DvrError::NoReply) => {
                Ok(RebootAck::LinkClosed)
            }
            Err(other) => Err(other),
        }
    }

    pub async fn restore_defaults(&mut self) -> Result<i64, DvrError> {
        let reply = self
            .request(msg::OP_MACHINE, default_config_body(self.session()))
            .await?;
        require_ok(&reply)
    }

    pub async fn ping(
        &mut self,
        url: &str,
        count: i64,
        timeout_secs: i64,
    ) -> Result<PingReport, DvrError> {
        let reply = self
            .request(
                msg::PING,
                ping_body(self.session(), url, count, timeout_secs),
            )
            .await?;
        require_ok(&reply)?;
        Ok(parse_ping(url, count, &reply))
    }

    pub async fn all_users(&mut self) -> Result<Vec<DeviceUser>, DvrError> {
        let reply = self.named(msg::USERS_GET, "GetAllUser").await?;
        Ok(parse_users(&reply))
    }

    pub async fn raw_users(&mut self) -> Result<Vec<Value>, DvrError> {
        let reply = self.named(msg::USERS_GET, "GetAllUser").await?;
        Ok(reply
            .get("Users")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn modify_password(
        &mut self,
        user: &str,
        old: &str,
        new: &str,
    ) -> Result<i64, DvrError> {
        let reply = self
            .request(
                msg::MODIFY_PASSWORD,
                modify_password_body(self.session(), user, old, new),
            )
            .await?;
        require_ok(&reply)
    }

    pub async fn rename_user(&mut self, current: &str, new_name: &str) -> Result<i64, DvrError> {
        let users = self.raw_users().await?;
        let user = users
            .iter()
            .find(|user| user.get("Name").and_then(Value::as_str) == Some(current))
            .cloned()
            .ok_or_else(|| {
                DvrError::Malformed(format!("the device has no user named {current}"))
            })?;
        let reply = self
            .request(
                msg::MODIFY_USER,
                modify_user_body(self.session(), current, &user, new_name),
            )
            .await?;
        require_ok(&reply)
    }

    pub async fn safety(&mut self, name: &str, inner: Option<Value>) -> Result<Value, DvrError> {
        let reply = self
            .request(msg::SAFETY, safety_body(self.session(), name, inner))
            .await?;
        require_ok(&reply)?;
        Ok(reply.get(name).cloned().unwrap_or(Value::Null))
    }

    pub async fn version_request(&mut self, language: &str) -> Result<Value, DvrError> {
        let reply = self
            .request(
                msg::VERSION_REQ,
                version_request_body(self.session(), language),
            )
            .await?;
        require_ok(&reply)?;
        Ok(reply)
    }

    pub async fn start_cloud_upgrade(
        &mut self,
        serial: &str,
        dev_id: &str,
        file_name: &str,
        version_date: &str,
    ) -> Result<i64, DvrError> {
        let body =
            start_cloud_upgrade_body(self.session(), serial, dev_id, file_name, version_date);
        let reply = self.request(msg::START_UPGRADE, body).await?;
        require_ok(&reply)
    }

    pub async fn upgrade_from_file(&mut self, image: &[u8]) -> Result<i64, DvrError> {
        let start = file_upgrade_body(self.session(), "Start", image.len());
        let reply = self.request(msg::SYSTEM_UPGRADE, start).await?;
        require_ok(&reply)?;
        let chunks = image.chunks(UPGRADE_CHUNK).count();
        for (index, chunk) in image.chunks(UPGRADE_CHUNK).enumerate() {
            self.send_frame_on(0, index + 1 == chunks, msg::UPGRADE_DATA, chunk)
                .await?;
        }
        self.send_frame_on(0, true, msg::UPGRADE_DATA, &[]).await?;
        let reply = self.await_reply(8).await?;
        require_ok(&reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvrip::fake::{connect, json_reply, serve, serve_scripted};

    fn sent(raw: &str) -> Value {
        serde_json::from_str(raw.trim_end_matches(['\0', '\n'])).unwrap()
    }

    #[test]
    fn reboot_body_matches_the_app() {
        let body = reboot_body(0x11);
        assert_eq!(
            body,
            json!({"Name":"OPMachine","OPMachine":{"Action":"Reboot"},"SessionID":"0x00000011"})
        );
    }

    #[test]
    fn default_config_sets_every_section_the_app_sets() {
        let body = default_config_body(1);
        let sections = body["OPDefaultConfig"].as_object().unwrap();
        assert_eq!(sections.len(), 10);
        for name in [
            "Account",
            "Alarm",
            "CameraPARAM",
            "CommPtz",
            "Encode",
            "General",
            "NetCommon",
            "NetServer",
            "Preview",
            "Record",
        ] {
            assert_eq!(sections[name], json!(1), "{name}");
        }
        assert_eq!(body["Name"], "OPDefaultConfig");
    }

    #[test]
    fn time_zone_uses_the_negated_minutes_the_app_sends() {
        assert_eq!(
            time_zone_value(420),
            json!({"FirstUserTimeZone":0,"timeMin":-420})
        );
        assert_eq!(minutes_east_of_utc(&json!({"timeMin":-420})), Some(420));
        assert_eq!(minutes_east_of_utc(&json!({"timeMin":300})), Some(-300));
    }

    #[test]
    fn device_time_shape_is_strict() {
        assert!(valid_device_time("2026-10-30 10:11:12"));
        assert!(!valid_device_time("2026-10-30T10:11:12"));
        assert!(!valid_device_time("2026-10-30 10:11"));
        assert!(!valid_device_time("2026-1a-30 10:11:12"));
    }

    #[test]
    fn firmware_dev_id_joins_the_fourth_part_onwards() {
        assert_eq!(
            firmware_dev_id("V5.01.R02.ABCDEF01.10010.123456.0000000").as_deref(),
            Some("ABCDEF01100101234560000000")
        );
        assert_eq!(firmware_dev_id("V5.01.R02.ABCDEF01.10010"), None);
        assert_eq!(
            firmware_build_date("2024-05-16 10:20:30").as_deref(),
            Some("2024-05-16")
        );
        assert_eq!(firmware_build_date("garbage"), None);
    }

    #[test]
    fn users_never_carry_the_password_hash() {
        let reply = json!({"Ret":100,"Users":[{"Name":"admin","Password":"tlJwpbo6","Group":"admin","Memo":"m","Reserved":true,"Sharable":true,"AuthorityList":["A","B"]}]});
        let users = parse_users(&reply);
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "admin");
        assert_eq!(users[0].authorities, 2);
        let text = serde_json::to_string(&users).unwrap();
        assert!(!text.contains("tlJwpbo6"));
        assert!(!text.to_lowercase().contains("password"));
    }

    #[test]
    fn ability_reads_nested_flags() {
        let abilities = json!({"OtherFunction":{"SupportCfgCloudupgrade":true,"SupportX":false},"NetServerFunction":{"WifiRouteSignalLevel":true}});
        assert!(ability(&abilities, "OtherFunction/SupportCfgCloudupgrade"));
        assert!(!ability(&abilities, "OtherFunction/SupportX"));
        assert!(!ability(&abilities, "Missing/Flag"));
        assert_eq!(
            ability_flags(&abilities),
            vec![
                "NetServerFunction/WifiRouteSignalLevel".to_string(),
                "OtherFunction/SupportCfgCloudupgrade".to_string()
            ]
        );
    }

    #[test]
    fn hex_ipv4_reads_the_little_endian_wire_form() {
        assert_eq!(hex_ipv4("0x0101A8C0").as_deref(), Some("192.168.1.1"));
        assert_eq!(hex_ipv4("192.168.1.1"), None);
    }

    #[test]
    fn uptime_reads_hex_and_decimal() {
        assert_eq!(uptime_minutes(&json!("0x0000003C")), Some(60));
        assert_eq!(uptime_minutes(&json!(61)), Some(61));
        assert_eq!(uptime_minutes(&json!("x")), None);
    }

    #[tokio::test]
    async fn modify_password_sends_sofia_hashes_on_1488() {
        let (address, handle) = serve(vec![json_reply(json!({"Ret":100}))]).await;
        let mut connection = connect(&address).await;
        connection.set_session(0x22);
        connection
            .modify_password("admin", "old-secret", "new-secret")
            .await
            .unwrap();
        drop(connection);
        let seen = handle.await.unwrap();
        let body = sent(&seen[0]);
        assert_eq!(body["EncryptType"], "MD5");
        assert_eq!(body["UserName"], "admin");
        assert_eq!(body["PassWord"], sofia_hash("old-secret"));
        assert_eq!(body["NewPassWord"], sofia_hash("new-secret"));
        assert!(!seen[0].contains("new-secret"));
        assert_eq!(body["SessionID"], "0x00000022");
    }

    #[tokio::test]
    async fn every_op_uses_its_message_id() {
        let (address, handle) = serve_scripted(|_, id, body| {
            let text = String::from_utf8_lossy(body).to_string();
            let reply = if text.contains("GetAllUser") {
                json!({"Ret":100,"Users":[{"Name":"admin","Group":"admin","AuthorityList":[]}]})
            } else if text.contains("OPTimeQuery") {
                json!({"Ret":100,"OPTimeQuery":"2026-10-30 10:11:12"})
            } else {
                json!({"Ret":100,"Ping":{"Result":true,"Avdelay":12,"Lost":0,"Num":3}})
            };
            vec![(id + 1, json_reply(reply))]
        })
        .await;
        let mut connection = connect(&address).await;
        connection.reboot().await.unwrap();
        connection.restore_defaults().await.unwrap();
        connection.set_time("2026-10-30 10:11:12").await.unwrap();
        assert_eq!(
            connection.time_query().await.unwrap().as_deref(),
            Some("2026-10-30 10:11:12")
        );
        let ping = connection.ping("8.8.8.8", 3, 2).await.unwrap();
        assert!(ping.reachable);
        assert_eq!(ping.average_ms, Some(12));
        connection.rename_user("admin", "keeper").await.unwrap();
        connection.version_request("English").await.unwrap();
        connection
            .start_cloud_upgrade("serial-one", "DEV", "fw.bin", "2026-01-02")
            .await
            .unwrap();
        connection
            .safety("SetNewPassword", Some(new_password_value("admin", "pw")))
            .await
            .unwrap();
        drop(connection);
        let seen = handle.await.unwrap();
        let ids: Vec<u16> = seen.iter().map(|(_, id, _)| *id).collect();
        assert_eq!(
            ids,
            vec![1450, 1450, 1450, 1452, 1052, 1472, 1484, 2016, 2018, 1650]
        );
        let bodies: Vec<Value> = seen
            .iter()
            .map(|(_, _, body)| sent(&String::from_utf8_lossy(body)))
            .collect();
        assert_eq!(bodies[0]["OPMachine"], json!({"Action":"Reboot"}));
        assert_eq!(bodies[1]["Name"], "OPDefaultConfig");
        assert_eq!(bodies[2]["OPTimeSetting"], "2026-10-30 10:11:12");
        assert_eq!(
            bodies[4]["Ping"],
            json!({"Num":3,"Timeout":2,"URL":"8.8.8.8"})
        );
        assert_eq!(bodies[6]["Name"], "ModifyUser");
        assert_eq!(bodies[6]["UserName"], "admin");
        assert_eq!(bodies[6]["User"]["Name"], "keeper");
        assert_eq!(
            bodies[7]["OPVersionReq"],
            json!({"Expect":0,"Language":"English","Manual":1})
        );
        assert_eq!(
            bodies[8]["OPStartUpgradeReq"],
            json!({"UUID":"serial-one","DevID":"DEV","FileName":"fw.bin","VerDate":"2026-01-02","Manual":1})
        );
        assert_eq!(
            bodies[9],
            json!({"Name":"SetNewPassword","SessionId":"0x00000000","SetNewPassword":{"UserName":"admin","NewPassword":"pw"}})
        );
    }

    #[tokio::test]
    async fn reboot_counts_a_closed_link_as_accepted() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = [0u8; crate::dvrip::HEADER_LEN];
            socket.read_exact(&mut header).await.unwrap();
            let header = crate::dvrip::frame::Header::decode(&header).unwrap();
            let mut body = vec![0u8; header.length as usize];
            socket.read_exact(&mut body).await.unwrap();
            (header.id, String::from_utf8_lossy(&body).into_owned())
        });
        let mut connection = connect(&address).await;
        let ack = connection.reboot().await.unwrap();
        assert_eq!(ack, RebootAck::LinkClosed);
        let (id, body) = handle.await.unwrap();
        assert_eq!(id, 1450);
        assert_eq!(sent(&body)["OPMachine"]["Action"], "Reboot");
    }

    #[tokio::test]
    async fn reboot_reports_a_refusal() {
        let (address, _handle) = serve(vec![json_reply(json!({"Ret":107}))]).await;
        let mut connection = connect(&address).await;
        let error = connection.reboot().await.unwrap_err();
        assert!(matches!(error, DvrError::Rejected { ret: 107, .. }));
    }

    #[tokio::test]
    async fn upgrade_from_file_streams_the_image_after_start() {
        let image = vec![7u8; UPGRADE_CHUNK + 10];
        let (address, handle) = serve_scripted(|_, id, _| match id {
            1520 => vec![(1521, json_reply(json!({"Ret":100})))],
            1522 => Vec::new(),
            _ => Vec::new(),
        })
        .await;
        let mut connection = connect(&address).await;
        let start = file_upgrade_body(connection.session(), "Start", image.len());
        assert_eq!(
            start["OPSystemUpgrade"],
            json!({"Action":"Start","Type":"System","FileSize":UPGRADE_CHUNK + 10})
        );
        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(300),
            connection.upgrade_from_file(&image),
        )
        .await;
        assert!(outcome.is_err());
        drop(connection);
        let seen = handle.await.unwrap();
        let ids: Vec<u16> = seen.iter().map(|(_, id, _)| *id).collect();
        assert_eq!(ids, vec![1520, 1522, 1522, 1522]);
        assert_eq!(seen[1].2.len(), UPGRADE_CHUNK);
        assert_eq!(seen[2].2.len(), 10);
        assert!(seen[3].2.is_empty());
    }
}
