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
    tokens: crate::cloud::DeviceTokenStore,
}

impl CloudProvider {
    pub fn new(cfg: &Config) -> Self {
        Self::with_identity(cfg, Identity::resolve())
    }

    pub fn with_identity(cfg: &Config, identity: Identity) -> Self {
        Self {
            client: EnvelopeClient::with_base(identity, cfg.cloud_base.clone()),
            status: StatusClient::new(STATUS_HOST),
            tokens: crate::cloud::DeviceTokenStore::load(None),
        }
    }

    fn token<'a>(&self, session: &'a VendorSession) -> Result<&'a str, ProviderError> {
        session.token.as_deref().ok_or_else(|| {
            ProviderError::Misconfigured(
                "the session carries no cloud token; sign in again".to_string(),
            )
        })
    }
    async fn device_login(
        &self,
        connection: &mut crate::dvrip::client::Connection,
        entry: &crate::cloud::types::CloudDevice,
    ) -> Result<String, ProviderError> {
        let serial = entry.uuid.trim().to_string();
        let control = crate::media::dhav::MediaCipher::channel();
        let mut note = String::from("no stored device token");
        if let Some(token) = self.tokens.get(&serial) {
            match self.token_login(connection, &token, &control).await {
                Ok(100) => {
                    return Ok(format!(
                        "stored device token ret 100, session 0x{:08x}",
                        connection.session()
                    ))
                }
                Ok(ret) => note = format!("stored device token ret {ret}"),
                Err(error) => note = format!("stored device token failed: {error}"),
            }
        }
        let user = match std::env::var("PROVIEW_DEVICE_USER")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            Some(name) => name,
            None => entry.username.trim().to_string(),
        };
        let pass = std::env::var("PROVIEW_DEVICE_PASS").unwrap_or_default();
        if user.is_empty() || pass.is_empty() {
            return Err(ProviderError::Misconfigured(format!(
                "{note}; registering a device token needs PROVIEW_DEVICE_USER and PROVIEW_DEVICE_PASS"
            )));
        }
        let reply = connection
            .login_reply(&user, &pass)
            .await
            .map_err(|error| {
                ProviderError::Upstream(format!("{note}, password registration failed: {error:?}"))
            })?;
        let issued = reply
            .get("AdminToken")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let stored = match issued {
            Some(token) => self.tokens.put(&serial, token).is_ok(),
            None => false,
        };
        Ok(format!(
            "{note}, password registration ret 100, device token stored {stored}, session 0x{:08x}",
            connection.session()
        ))
    }

    async fn token_login(
        &self,
        connection: &mut crate::dvrip::client::Connection,
        token: &str,
        control: &crate::media::dhav::MediaCipher,
    ) -> Result<i64, String> {
        connection.set_session(CLAIM_SESSION);
        let pre_claim = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Claim",
                "Parameter": {
                    "Channel": 0,
                    "CombinMode": "CONNECT_ALL",
                    "StreamType": "Main",
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
            .map_err(|error| format!("pre-claim failed: {error:?}"))?;
        let caps = await_frame(connection, MONITOR_CLAIM_REPLY, 5).await;
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
        let communicate_key = match sealed_from.as_deref() {
            Some(pair) => {
                let (modulus, exponent) = pair.split_once(',').unwrap_or((pair, "010001"));
                crate::dvrip::crypto::communicate_key(
                    modulus.trim(),
                    exponent.trim(),
                    &rps_client::session_key(),
                )?
            }
            None => String::new(),
        };
        let login_body = serde_json::json!({
            "EncryptType": "TOKEN",
            "LoginType": "DVRIP-FutureHome",
            "UserName": token,
            "PassWord": "",
            "CommunicateKey": communicate_key,
            "User-Agent": format!("Funv1.2.2c-And-{}", rps_client::session_key()),
        });
        connection
            .send_encrypted(
                control,
                crate::dvrip::msg::MONITOR_CHANNEL,
                crate::dvrip::msg::LOGIN,
                &login_body,
            )
            .await
            .map_err(|error| format!("encrypted login failed: {error:?}"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(2), connection.recv()).await;
            let Ok(Ok((_, payload))) = frame else {
                break;
            };
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
                if let Some(ret) = reply.get("Ret").and_then(serde_json::Value::as_i64) {
                    return Ok(ret);
                }
            }
        }
        Err("the device answered no login reply".to_string())
    }
}

impl CloudProvider {
    pub async fn open_media(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
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
        let mut connection = tunnel.into_connection();
        self.device_login(&mut connection, entry).await?;
        let device_channel = channel.saturating_sub(1) as u8;
        let stream = if high { "Main" } else { "Sub" };
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
            .map_err(|error| ProviderError::Upstream(format!("monitor claim failed: {error:?}")))?;
        let _ = await_frame(&mut connection, MONITOR_CLAIM_REPLY, 5).await;
        let start = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Start",
                "Parameter": {
                    "Channel": device_channel,
                    "CombinMode": "NONE",
                    "StreamType": stream,
                    "TransMode": "TCP",
                },
            },
            "SessionID": format!("0x{:08x}", connection.session()),
        });
        connection
            .send_plain(0, crate::dvrip::msg::MONITOR_START, &start)
            .await
            .map_err(|error| ProviderError::Upstream(format!("monitor start failed: {error:?}")))?;
        Ok(connection)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn pump_live(
        hub: std::sync::Arc<crate::live::LiveHub>,
        key: crate::live::StreamKey,
        cfg: Config,
        session: VendorSession,
        device: String,
        channel: u32,
        high: bool,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let provider = CloudProvider::new(&cfg);
        tracing::info!(device = %device, "cloud live source starting");
        let outcome = provider
            .pump_inner(
                hub.clone(),
                key.clone(),
                &session,
                &device,
                channel,
                high,
                &stop,
            )
            .await;
        match &outcome {
            Ok(()) => tracing::info!(device = %device, "cloud live source finished"),
            Err(error) => tracing::warn!(%error, device = %device, "cloud live source stopped"),
        }
        hub.publisher_done(&key);
    }

    #[allow(clippy::too_many_arguments)]
    async fn pump_inner(
        &self,
        hub: std::sync::Arc<crate::live::LiveHub>,
        key: crate::live::StreamKey,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<(), ProviderError> {
        let mut connection = self.open_media(session, device, channel, high).await?;
        hub.set_meta(
            &key,
            crate::live::StreamMeta {
                video_codec: Some("H265".to_string()),
                ..crate::live::StreamMeta::default()
            },
        );
        let mut demux = crate::media::live::LiveDemux::new();
        let mut buffered: Vec<u8> = Vec::new();
        let mut parameter_sets: Vec<Vec<u8>> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while parameter_sets.len() < 3 && std::time::Instant::now() < deadline {
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(6), connection.recv()).await;
            let Ok(Ok((header, payload))) = frame else {
                break;
            };
            if header.id != crate::media::live::LIVE_MEDIA_ID {
                continue;
            }
            let bytes = demux.push(&payload);
            if bytes.is_empty() {
                continue;
            }
            buffered.extend_from_slice(&bytes);
            parameter_sets = parameter_sets_of(&buffered);
        }
        if parameter_sets.len() < 3 {
            return Err(ProviderError::Upstream(format!(
                "the cloud stream carried {} of 3 HEVC parameter sets in 10 s",
                parameter_sets.len()
            )));
        }
        let (mut stdin, reader, child) =
            crate::source::spawn_fmp4_publisher(&hub, &key, "H265", &parameter_sets)
                .await
                .map_err(|error| ProviderError::Upstream(error.to_string()))?;
        tokio::io::AsyncWriteExt::write_all(&mut stdin, &buffered)
            .await
            .map_err(|error| ProviderError::Upstream(format!("ffmpeg stdin: {error}")))?;
        buffered.clear();
        let mut written = 0usize;
        loop {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(10), connection.recv()).await;
            match frame {
                Ok(Ok((header, payload))) => {
                    if header.id != crate::media::live::LIVE_MEDIA_ID {
                        continue;
                    }
                    let bytes = demux.push(&payload);
                    if bytes.is_empty() {
                        continue;
                    }
                    written += bytes.len();
                    if tokio::io::AsyncWriteExt::write_all(&mut stdin, &bytes)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        drop(stdin);
        let stderr = crate::source::drain_stderr(child).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), reader).await;
        if let Some(text) = stderr {
            tracing::debug!(%text, "cloud ffmpeg stderr");
        }
        let stats = demux.stats();
        tracing::info!(
            device,
            written,
            keyframes = stats.keyframes,
            deltas = stats.deltas,
            dropped = stats.dropped,
            "cloud live source closed"
        );
        Ok(())
    }
}

fn parameter_sets_of(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut sets: Vec<Vec<u8>> = Vec::new();
    let mut index = 0usize;
    while index + 3 < bytes.len() {
        if bytes[index] == 0 && bytes[index + 1] == 0 && bytes[index + 2] == 1 {
            let start = index + 3;
            let mut end = bytes.len();
            let mut probe = start;
            while probe + 3 < bytes.len() {
                if bytes[probe] == 0 && bytes[probe + 1] == 0 && bytes[probe + 2] == 1 {
                    end = probe;
                    break;
                }
                probe += 1;
            }
            let nal = &bytes[start..end];
            let kind = crate::media::hevc::nal_type(nal);
            if matches!(kind, Some(32) | Some(33) | Some(34))
                && !sets.iter().any(|existing| existing == nal)
            {
                sets.push(nal.to_vec());
            }
            index = end;
            continue;
        }
        index += 1;
    }
    sets
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
        _session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        if device.trim().is_empty() || device.contains('/') || device.contains("..") {
            return Err(ProviderError::UnknownDevice(device.to_string()));
        }
        let quality = if high { "main" } else { "sub" };
        Ok(StreamTicket {
            kind: StreamKind::Ws,
            url: format!("/ws/live?device={device}&channel={channel}&quality={quality}"),
            expires_in: 120,
            source: ProviderSource::Cloud,
        })
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

    #[tokio::test]
    async fn the_ticket_points_at_the_live_socket_and_names_the_cloud() {
        let provider = CloudProvider::with_identity(
            &crate::config::Config::from_env(),
            Identity::new("uuid", "key", "secret"),
        );
        let session = VendorSession {
            account: "probe".to_string(),
            token: Some("access".to_string()),
        };
        let ticket = provider
            .stream(&session, "7f92970ddf5282743hrc", 1, true)
            .await
            .expect("ticket");
        assert!(matches!(ticket.kind, StreamKind::Ws));
        assert_eq!(ticket.source, ProviderSource::Cloud);
        assert_eq!(
            ticket.url,
            "/ws/live?device=7f92970ddf5282743hrc&channel=1&quality=main"
        );
    }

    #[tokio::test]
    async fn a_path_like_device_is_refused_before_a_ticket() {
        let provider = CloudProvider::with_identity(
            &crate::config::Config::from_env(),
            Identity::new("uuid", "key", "secret"),
        );
        let session = VendorSession {
            account: "probe".to_string(),
            token: Some("access".to_string()),
        };
        let error = provider
            .stream(&session, "../etc/passwd", 1, true)
            .await
            .expect_err("a path is not a device");
        assert!(matches!(error, ProviderError::UnknownDevice(_)));
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
