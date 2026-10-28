use super::rps as rps_client;
use super::{
    p2p, Alarm, CloudPath, Device, DeviceSettings, Provider, ProviderError, ProviderSource,
    Recording, StreamKind, StreamTicket, VendorSession,
};
use crate::cloud::envelope::{EnvelopeClient, Identity, MISSING_CLOUD_IDENTITY};
use crate::cloud::status::{self, StatusClient, STATUS_HOST};
use crate::cloud::CloudError;

const MONITOR_CLAIM_REPLY: u16 = 0x0586;
const CLAIM_SESSION: u32 = 0x0001_869f;

use crate::config::Config;

pub struct CloudProvider {
    client: EnvelopeClient,
    status: StatusClient,
}

impl CloudProvider {
    pub fn new(cfg: &Config) -> Self {
        Self::with_identity(cfg, Identity::resolve())
    }

    pub fn with_identity(cfg: &Config, identity: Identity) -> Self {
        Self {
            client: EnvelopeClient::with_base(identity, cfg.cloud_base.clone()),
            status: StatusClient::new(STATUS_HOST),
        }
    }

    fn token<'a>(&self, session: &'a VendorSession) -> Result<&'a str, ProviderError> {
        session.token.as_deref().ok_or_else(|| {
            ProviderError::Misconfigured(
                "the session carries no cloud token; sign in again".to_string(),
            )
        })
    }
}

async fn await_frame(
    connection: &mut crate::dvrip::client::Connection,
    id: u16,
    seconds: u64,
) -> Option<Vec<u8>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    while std::time::Instant::now() < deadline {
        let frame =
            tokio::time::timeout(std::time::Duration::from_secs(2), connection.recv()).await;
        match frame {
            Ok(Ok((header, payload))) if header.id == id => return Some(payload),
            Ok(Ok(_)) => continue,
            _ => return None,
        }
    }
    None
}

fn map_cloud(error: CloudError) -> ProviderError {
    match error {
        CloudError::Credentials => ProviderError::BadCredentials,
        CloudError::Rejected(message) => ProviderError::Upstream(message),
        CloudError::Expired => ProviderError::Expired,
        CloudError::Unreachable(message) => ProviderError::Unreachable(message),
        CloudError::Malformed(message) => ProviderError::Upstream(message),
    }
}

#[async_trait::async_trait]
impl Provider for CloudProvider {
    fn source(&self) -> ProviderSource {
        ProviderSource::Cloud
    }

    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError> {
        if !self.client.identity().is_complete() {
            return Err(ProviderError::Misconfigured(
                MISSING_CLOUD_IDENTITY.to_string(),
            ));
        }
        let tokens = self
            .client
            .login_envelope(user, pass)
            .await
            .map_err(map_cloud)?;
        Ok(VendorSession {
            account: user.to_string(),
            token: Some(tokens.access),
        })
    }

    async fn devices(&self, session: &VendorSession) -> Result<Vec<Device>, ProviderError> {
        let token = self.token(session)?;
        let devices = self.client.device_list(token).await.map_err(map_cloud)?;
        let mut pending = tokio::task::JoinSet::new();
        for (index, device) in devices.iter().enumerate() {
            let status = self.status.clone();
            let serial = device.uuid.clone();
            let code = device.auth_code();
            pending.spawn(async move {
                let state = status.query(&serial, &code).await;
                (index, state)
            });
        }
        let mut states = vec![None; devices.len()];
        while let Some(joined) = pending.join_next().await {
            if let Ok((index, state)) = joined {
                states[index] = Some(match state {
                    Ok(value) => value,
                    Err(error) => status::unchecked(error.to_string()),
                });
            }
        }
        Ok(devices
            .iter()
            .enumerate()
            .map(|(index, device)| device.to_provider(states[index].as_ref()))
            .collect())
    }

    async fn cloud_path(
        &self,
        session: &VendorSession,
        device: &str,
    ) -> Result<CloudPath, ProviderError> {
        let token = self.token(session)?;
        let devices = self.client.device_list(token).await.map_err(map_cloud)?;
        let Some(entry) = devices.iter().find(|entry| entry.uuid == device.trim()) else {
            return Err(ProviderError::UnknownDevice(device.to_string()));
        };
        let rps = entry
            .extra
            .get("rps")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let state = match self
            .status
            .query(&entry.uuid, &status::auth_code_of(entry))
            .await
        {
            Ok(state) => state,
            Err(error) => status::unchecked(error.to_string()),
        };
        let mut report = CloudPath {
            online: state.online,
            detail: state.summary(),
            ..CloudPath::default()
        };
        if rps.is_empty() {
            report.detail = format!("{} (the device list carries no rps id)", report.detail);
            return Ok(report);
        }
        let Some((host, port)) = state.access_server() else {
            report.detail = format!("{} (no cluster reported an access server)", report.detail);
            return Ok(report);
        };
        report.access = Some(format!("{host}:{port}"));

        let client_token = rps_client::client_token();
        let session_id = rps_client::session_id();
        let request = p2p::agent_request(
            &rps,
            &client_token,
            &entry.uuid,
            p2p::AGENT_SERVICE_MEDIA,
            &session_id,
            p2p::AGENT_STEP_MEDIA,
        );
        let agent = match rps_client::AccessClient::new()
            .assign(&host, port, &request)
            .await
        {
            Ok(agent) => agent,
            Err(error) => {
                report.relay_error = Some(error.to_string());
                report.detail = format!("{} (agent assignment failed)", report.detail);
                return Ok(report);
            }
        };
        report.agent = Some(format!("{}:{}", agent.host, agent.port));
        let frame = p2p::handshake_frame(&rps, &client_token, &entry.uuid, &session_id);
        let outcome = rps_client::open(&agent, &frame)
            .await
            .map(|tunnel| tunnel.reply)
            .map_err(|error| error.to_string());
        apply_relay(&mut report, outcome);
        Ok(report)
    }

    async fn stream(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        let token = self.token(session)?;
        let devices = self.client.device_list(token).await.map_err(map_cloud)?;
        let Some(entry) = devices.iter().find(|entry| entry.uuid == device.trim()) else {
            return Err(ProviderError::UnknownDevice(device.to_string()));
        };
        let rps = entry
            .extra
            .get("rps")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if rps.is_empty() {
            return Err(ProviderError::Upstream(
                "the device list carries no rps id, so the relay cannot be asked".to_string(),
            ));
        }
        let state = match self
            .status
            .query(&entry.uuid, &status::auth_code_of(entry))
            .await
        {
            Ok(state) => state,
            Err(error) => status::unchecked(error.to_string()),
        };
        let Some((host, port)) = state.access_server() else {
            return Err(ProviderError::Upstream(format!(
                "{} (no cluster reported an access server)",
                state.summary()
            )));
        };
        let client_token = rps_client::client_token();
        let session_id = rps_client::session_id();
        let request = p2p::agent_request(
            &rps,
            &client_token,
            &entry.uuid,
            p2p::AGENT_SERVICE_MEDIA,
            &session_id,
            p2p::AGENT_STEP_MEDIA,
        );
        let agent = rps_client::AccessClient::new()
            .assign(&host, port, &request)
            .await
            .map_err(|error| {
                ProviderError::Upstream(format!("agent assignment failed: {error}"))
            })?;
        let frame = p2p::handshake_frame(&rps, &client_token, &entry.uuid, &session_id);
        let tunnel = rps_client::open(&agent, &frame)
            .await
            .map_err(|error| ProviderError::Upstream(format!("relay handshake failed: {error}")))?;
        if !tunnel.reply.accepted() {
            return Err(ProviderError::Upstream(format!(
                "relay refused the session: ErrorNum {}",
                tunnel.reply.error_num
            )));
        }
        let dest = tunnel.reply.dest_ip.clone();
        let mut connection = tunnel.into_connection();
        let access = self.token(session)?.to_string();
        let device_id = entry
            .extra
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ProviderError::Upstream(format!(
                    "relay accepted for {dest}, the device list carries no id to request a device token for"
                ))
            })?;
        let device_token = match std::env::var("PROVIEW_TOKEN_SOURCE").as_deref() {
            Ok("env") => std::env::var("PROVIEW_DEVICE_TOKEN").map_err(|_| {
                ProviderError::Misconfigured(
                    "PROVIEW_TOKEN_SOURCE=env needs PROVIEW_DEVICE_TOKEN".to_string(),
                )
            })?,
            Ok("mdlist") => entry
                .extra
                .get("deviceToken")
                .and_then(|value| value.get("AdminToken"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    ProviderError::Upstream(format!(
                        "relay accepted for {dest}, the device list carries no AdminToken"
                    ))
                })?,
            _ => self
                .client
                .device_token(&access, device_id)
                .await
                .map_err(|error| {
                    ProviderError::Upstream(format!("account device token failed: {error}"))
                })?,
        };
        let device_user = entry.username.trim();
        if device_user.is_empty() {
            return Err({
                ProviderError::Upstream(format!(
                    "relay accepted for {dest}, the device list carries no username to log in with"
                ))
            });
        }
        let device_channel = channel.saturating_sub(1) as u8;
        let stream = if high { "Main" } else { "Sub" };
        let control = crate::media::dhav::MediaCipher::channel();
        connection.set_session(CLAIM_SESSION);
        let pre_claim = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Claim",
                "Parameter": {
                    "Channel": device_channel,
                    "CombinMode": "CONNECT_ALL",
                    "StreamType": stream,
                    "TransMode": "TCP",
                },
            },
            "DHParameter": { "RandomStrA": crate::provider::rps::random_lower(4) },
            "SessionID": format!("0x{CLAIM_SESSION:08x}"),
        });
        connection
            .send_plain(
                crate::dvrip::msg::MONITOR_CHANNEL,
                crate::dvrip::msg::MONITOR,
                &pre_claim,
            )
            .await
            .map_err(|error| {
                ProviderError::Upstream(format!(
                    "relay accepted for {dest}, pre-claim failed: {error:?}"
                ))
            })?;
        let caps = await_frame(&mut connection, MONITOR_CLAIM_REPLY, 5).await;
        let device_key = caps
            .as_deref()
            .and_then(|payload| control.decrypt_body(payload))
            .and_then(|raw| {
                serde_json::from_slice::<serde_json::Value>(crate::media::dhav::trim_nul(&raw)).ok()
            })
            .and_then(|value| {
                value
                    .get("PublicKey")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });
        connection.set_session(0);
        let sealed_from = device_key.clone().or_else(|| {
            std::env::var("PROVIEW_DEVICE_MODULUS")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|modulus| format!("{modulus},010001"))
        });
        let mut seal_error = String::from("no device key");
        let communicate_key = sealed_from
            .as_deref()
            .map(|pair| {
                let (modulus, exponent) = pair.split_once(',').unwrap_or((pair, "010001"));
                match crate::dvrip::crypto::communicate_key(
                    modulus.trim(),
                    exponent.trim(),
                    &rps_client::session_key(),
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        seal_error = error;
                        String::new()
                    }
                }
            })
            .unwrap_or_default();
        let login_body = serde_json::json!({
            "EncryptType": "TOKEN",
            "LoginType": "DVRIP-FutureHome",
            "UserName": device_token,
            "PassWord": "",
            "CommunicateKey": communicate_key,
            "User-Agent": format!("Funv1.2.2c-And-{}", rps_client::session_key()),
        });
        connection
            .send_encrypted(
                &control,
                crate::dvrip::msg::MONITOR_CHANNEL,
                crate::dvrip::msg::LOGIN,
                &login_body,
            )
            .await
            .map_err(|error| {
                ProviderError::Upstream(format!(
                    "relay accepted for {dest}, encrypted login failed: {error:?}"
                ))
            })?;
        let caps_note = match caps.as_deref() {
            Some(payload) => format!(
                "{} bytes, opened {}, key {}",
                payload.len(),
                if control.decrypt_body(payload).is_some() {
                    "yes"
                } else {
                    "no"
                },
                if device_key.is_some() { "yes" } else { "no" }
            ),
            None => "missing".to_string(),
        };
        let mut ids: Vec<String> = Vec::new();
        let mut login_ret = String::from("-");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(2), connection.recv()).await;
            let Ok(Ok((header, payload))) = frame else {
                break;
            };
            ids.push(format!("0x{:04x}/{}", header.id, payload.len()));
            let body = crate::media::dhav::trim_nul(&payload);
            let plain = serde_json::from_slice::<serde_json::Value>(body)
                .ok()
                .or_else(|| {
                    control.decrypt_body(body).and_then(|raw| {
                        serde_json::from_slice::<serde_json::Value>(crate::media::dhav::trim_nul(
                            &raw,
                        ))
                        .ok()
                    })
                });
            if let Some(reply) = plain {
                if let Some(ret) = reply.get("Ret") {
                    login_ret = format!(
                        "{} {}",
                        ret,
                        String::from_utf8_lossy(crate::media::dhav::trim_nul(&payload))
                            .chars()
                            .take(140)
                            .collect::<String>()
                    );
                    break;
                }
            }
        }
        let login = format!(
            "caps {caps_note}, encrypted token login (token {} chars, communicate key {}) ret {login_ret} session 0x{:08x} frames [{}]",
            device_token.chars().count(),
            if communicate_key.is_empty() { format!("none: {seal_error}") } else if device_key.is_some() { "sealed from caps".to_string() } else { "sealed from env".to_string() },
            connection.session(),
            ids.join(" ")
        );
        let claim = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Claim",
                "Parameter": {
                    "Channel": device_channel,
                    "CombinMode": "CONNECT_ALL",
                    "StreamType": stream,
                    "TransMode": "TCP",
                },
            },
            "DHParameter": { "RandomStrA": crate::provider::rps::random_lower(4) },
            "SessionID": format!("0x{:08x}", connection.session()),
        });
        connection
            .send_plain(
                crate::dvrip::msg::MONITOR_CHANNEL,
                crate::dvrip::msg::MONITOR,
                &claim,
            )
            .await
            .map_err(|error| {
                ProviderError::Upstream(format!(
                    "relay accepted for {dest}, monitor claim failed: {error:?}"
                ))
            })?;
        let claim_seen = await_frame(&mut connection, MONITOR_CLAIM_REPLY, 5)
            .await
            .is_some();
        let claim = format!(
            "claim reply {} session 0x{:08x}",
            if claim_seen { "seen" } else { "missing" },
            connection.session()
        );
        let session = connection.session();
        let start = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Claim",
                "Action1": "Start",
                "Parameter": {
                    "Channel": device_channel,
                    "CombinMode": "NONE",
                    "StreamType": stream,
                    "TransMode": "TCP",
                },
            },
            "SessionID": format!("0x{session:08x}"),
        });
        connection
            .send_plain(0, crate::dvrip::msg::MONITOR, &start)
            .await
            .map_err(|error| ProviderError::Upstream(format!("monitor start failed: {error:?}")))?;
        let mut demux = crate::media::live::LiveDemux::new();
        let mut bytes = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(3), connection.recv()).await;
            match frame {
                Ok(Ok((header, payload))) if header.id == crate::media::live::LIVE_MEDIA_ID => {
                    bytes += demux.push(&payload).len();
                }
                Ok(Ok(_)) => {}
                _ => break,
            }
        }
        let stats = demux.stats();
        let _ = StreamKind::Rtsp;
        Err(ProviderError::Upstream(format!(
            "live media from the cloud relay for {dest}: {login}, {claim}, {} keyframes, {} deltas, {} dropped, {bytes} elementary bytes",
            stats.keyframes, stats.deltas, stats.dropped
        )))
    }

    async fn playback(
        &self,
        _session: &VendorSession,
        _device: &str,
        _channel: u32,
    ) -> Result<Vec<Recording>, ProviderError> {
        Err(ProviderError::NotImplemented(
            "cloud playback needs the CSS file-search envelope, which is not solved yet",
        ))
    }

    async fn alarms(
        &self,
        _session: &VendorSession,
        _limit: u32,
    ) -> Result<Vec<Alarm>, ProviderError> {
        Err(ProviderError::NotImplemented(
            "cloud alarms need the push channel on 7700-7706, which is not mapped yet",
        ))
    }

    async fn settings(
        &self,
        _session: &VendorSession,
        _device: &str,
    ) -> Result<DeviceSettings, ProviderError> {
        Err(ProviderError::NotImplemented(
            "cloud device settings need the CSS config envelope, which is not solved yet",
        ))
    }

    async fn save_settings(
        &self,
        _session: &VendorSession,
        _settings: &DeviceSettings,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::NotImplemented(
            "cloud device settings need the CSS config envelope, which is not solved yet",
        ))
    }
}

pub fn apply_relay(report: &mut CloudPath, outcome: Result<p2p::RelayReply, String>) {
    match outcome {
        Ok(reply) if reply.accepted() => {
            report.relay_accepted = true;
            report.dest_ip = Some(reply.dest_ip.clone());
            report.mtu = Some(reply.mtu.clone());
            report.detail = format!(
                "{} (relay accepted the session for {} at mtu {})",
                report.detail, reply.dest_ip, reply.mtu
            );
        }
        Ok(reply) => {
            report.relay_error = Some(format!("ErrorNum {}", reply.error_num));
            report.detail = format!(
                "{} (relay refused the session, ErrorNum {})",
                report.detail, reply.error_num
            );
        }
        Err(error) => {
            report.relay_error = Some(error);
            report.detail = format!("{} (relay handshake failed)", report.detail);
        }
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    fn reply(error_num: &str) -> p2p::RelayReply {
        p2p::RelayReply {
            error_num: error_num.to_string(),
            dest_ip: "203.0.113.9".to_string(),
            mtu: "1400".to_string(),
        }
    }

    #[test]
    fn an_accepted_relay_is_not_reported_as_an_error() {
        let mut report = CloudPath {
            detail: "cloud status online".to_string(),
            ..CloudPath::default()
        };
        apply_relay(&mut report, Ok(reply("200")));
        assert!(report.relay_accepted);
        assert_eq!(report.relay_error, None);
        assert_eq!(report.dest_ip.as_deref(), Some("203.0.113.9"));
        assert_eq!(report.mtu.as_deref(), Some("1400"));
        assert!(report.detail.contains("relay accepted the session"));
    }

    #[test]
    fn a_refused_relay_is_reported_with_its_code() {
        let mut report = CloudPath::default();
        apply_relay(&mut report, Ok(reply("400")));
        assert!(!report.relay_accepted);
        assert_eq!(report.relay_error.as_deref(), Some("ErrorNum 400"));
        assert!(report.detail.contains("relay refused the session"));
    }

    #[test]
    fn a_failed_handshake_is_reported_as_such() {
        let mut report = CloudPath::default();
        apply_relay(&mut report, Err("connection refused".to_string()));
        assert!(!report.relay_accepted);
        assert_eq!(report.relay_error.as_deref(), Some("connection refused"));
        assert!(report.detail.contains("relay handshake failed"));
    }
}
