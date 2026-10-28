use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::rps as rps_client;
use super::{
    p2p, Alarm, CloudPath, Device, DeviceSettings, Provider, ProviderError, ProviderSource,
    Recording, StreamKind, StreamTicket, VendorSession,
};
use crate::cloud::envelope::{EnvelopeClient, Identity, MISSING_CLOUD_IDENTITY};
use crate::cloud::status::{self, StatusClient, STATUS_HOST};
use crate::cloud::types::CloudDevice;
use crate::cloud::CloudError;

const MONITOR_CLAIM_REPLY: u16 = 0x0586;
const CLAIM_SESSION: u32 = 0x0001_869f;
const SESSION_ENTRY_TTL: Duration = Duration::from_secs(30);

use crate::config::Config;

struct SessionEntries {
    at: Instant,
    devices: Arc<Vec<CloudDevice>>,
}

pub struct CloudProvider {
    http: reqwest::Client,
    base: String,
    identity: Option<Identity>,
    counter: Arc<std::sync::atomic::AtomicU32>,
    status: StatusClient,
    tokens: crate::cloud::DeviceTokenStore,
    entries: Mutex<HashMap<String, SessionEntries>>,
    entry_ttl: Duration,
}

impl CloudProvider {
    pub fn new(cfg: &Config) -> Self {
        Self::with_identity_override(cfg, None)
    }

    pub fn with_identity(cfg: &Config, identity: Identity) -> Self {
        Self::with_identity_override(cfg, Some(identity))
    }

    fn with_identity_override(cfg: &Config, identity: Option<Identity>) -> Self {
        Self {
            http: crate::cloud::envelope::http(),
            base: cfg.cloud_base.clone(),
            identity,
            counter: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            status: StatusClient::new(STATUS_HOST),
            tokens: crate::cloud::DeviceTokenStore::load(None),
            entries: Mutex::new(HashMap::new()),
            entry_ttl: SESSION_ENTRY_TTL,
        }
    }

    fn envelope(&self) -> EnvelopeClient {
        let identity = self.identity.clone().unwrap_or_else(Identity::resolve);
        EnvelopeClient::with_shared(
            identity,
            self.base.clone(),
            self.http.clone(),
            self.counter.clone(),
        )
    }

    #[cfg(test)]
    fn with_entry_ttl(mut self, ttl: Duration) -> Self {
        self.entry_ttl = ttl;
        self
    }

    fn cached_entries(&self, session: &str) -> Option<Arc<Vec<CloudDevice>>> {
        let mut entries = match self.entries.lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        match entries.get(session) {
            Some(entry) if entry.at.elapsed() < self.entry_ttl => Some(entry.devices.clone()),
            Some(_) => {
                entries.remove(session);
                None
            }
            None => None,
        }
    }

    fn remember_entries(&self, session: &str, devices: Arc<Vec<CloudDevice>>) {
        let mut entries = match self.entries.lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.retain(|_, entry| entry.at.elapsed() < self.entry_ttl);
        entries.insert(
            session.to_string(),
            SessionEntries {
                at: Instant::now(),
                devices,
            },
        );
    }

    async fn fetch_entries(
        &self,
        session: &VendorSession,
    ) -> Result<Arc<Vec<CloudDevice>>, ProviderError> {
        let token = self.token(session)?;
        let devices = Arc::new(
            self.envelope()
                .device_list(token)
                .await
                .map_err(map_cloud)?,
        );
        self.remember_entries(token, devices.clone());
        Ok(devices)
    }

    fn token<'a>(&self, session: &'a VendorSession) -> Result<&'a str, ProviderError> {
        session.token.as_deref().ok_or_else(|| {
            ProviderError::Misconfigured(
                "the session carries no cloud token; sign in again".to_string(),
            )
        })
    }
    async fn device_login<F, Fut>(
        &self,
        mut connection: crate::dvrip::client::Connection,
        entry: &crate::cloud::types::CloudDevice,
        reopen: F,
    ) -> Result<(crate::dvrip::client::Connection, String), ProviderError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<crate::dvrip::client::Connection, ProviderError>>,
    {
        let serial = entry.uuid.trim().to_string();
        let control = crate::media::dhav::MediaCipher::channel();
        let mut note = String::from("no stored device token");
        if let Some(token) = self.tokens.get(&serial) {
            match self.token_login(&mut connection, &token, &control).await {
                Ok(100) => {
                    let note = format!(
                        "stored device token ret 100, session 0x{:08x}",
                        connection.session()
                    );
                    return Ok((connection, note));
                }
                Ok(ret) => {
                    let _ = self.tokens.forget(&serial);
                    note = format!("stored device token ret {ret}, token dropped");
                }
                Err(error) => {
                    let _ = self.tokens.forget(&serial);
                    note = format!("stored device token failed: {error}, token dropped");
                }
            }
            drop(connection);
            connection = reopen().await?;
        }
        let (user, pass) = match device_credential_for(&serial, entry) {
            DeviceCredential::Allowed { user, pass } => (user, pass),
            DeviceCredential::Refused(reason) => {
                return Err(ProviderError::Misconfigured(format!("{note}; {reason}")))
            }
        };
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
        let summary = format!(
            "{note}, password login ret 100, device token stored {stored}, session 0x{:08x}",
            connection.session()
        );
        Ok((connection, summary))
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
            let Ok(Ok((header, payload))) = frame else {
                break;
            };
            if header.id != crate::dvrip::msg::LOGIN_REPLY {
                continue;
            }
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
    pub async fn entry_of(
        &self,
        session: &VendorSession,
        device: &str,
    ) -> Result<Option<CloudDevice>, ProviderError> {
        let serial = device.trim();
        if let Some(devices) = self.cached_entries(self.token(session)?) {
            if let Some(found) = devices.iter().find(|entry| entry.uuid == serial) {
                return Ok(Some(found.clone()));
            }
        }
        let devices = self.fetch_entries(session).await?;
        Ok(devices.iter().find(|entry| entry.uuid == serial).cloned())
    }

    pub async fn owns(&self, session: &VendorSession, device: &str) -> Result<bool, ProviderError> {
        Ok(self.entry_of(session, device).await?.is_some())
    }

    async fn open_tunnel(
        &self,
        entry: &CloudDevice,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
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
        tracing::debug!(device = %entry.uuid, dest = %tunnel.reply.dest_ip, "cloud tunnel open");
        Ok(tunnel.into_connection())
    }

    pub async fn open_media(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
        let Some(entry) = self.entry_of(session, device).await? else {
            return Err(ProviderError::UnknownDevice(device.to_string()));
        };
        let connection = self.open_tunnel(&entry).await?;
        let (mut connection, login_note) = self
            .device_login(connection, &entry, || self.open_tunnel(&entry))
            .await?;
        tracing::debug!(device = %entry.uuid, %login_note, "cloud device login");
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
        let start = monitor_start_body(device_channel, stream, connection.session());
        start_monitor(&mut connection, &start)
            .await
            .map_err(|error| ProviderError::Upstream(format!("monitor start failed: {error:?}")))?;
        Ok(connection)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn pump_live(
        hub: std::sync::Arc<crate::live::LiveHub>,
        key: crate::live::StreamKey,
        provider: Arc<CloudProvider>,
        session: VendorSession,
        device: String,
        channel: u32,
        high: bool,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
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
            Ok(spawned) => {
                if *spawned {
                    hub.publisher_done(&key);
                }
                tracing::info!(device = %device, "cloud live source finished");
            }
            Err(error) => tracing::warn!(%error, device = %device, "cloud live source stopped"),
        }
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
    ) -> Result<bool, ProviderError> {
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
            return Err(ProviderError::Upstream(
                "the cloud stream did not carry VPS, SPS and PPS within 10 s".to_string(),
            ));
        }
        let (mut stdin, reader, child) =
            crate::source::spawn_fmp4_publisher(&hub, &key, "H265", &parameter_sets)
                .await
                .map_err(|error| ProviderError::Upstream(error.to_string()))?;
        let mut written = 0usize;
        let mut ffmpeg_open = tokio::io::AsyncWriteExt::write_all(&mut stdin, &buffered)
            .await
            .is_ok();
        buffered.clear();
        if !ffmpeg_open {
            tracing::warn!(
                device,
                "cloud ffmpeg closed its input before the first frame"
            );
        }
        while ffmpeg_open {
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
                        tracing::warn!(device, "cloud ffmpeg stopped reading");
                        ffmpeg_open = false;
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(device, error = ?error, "cloud live source read failed");
                    break;
                }
                Err(_) => {
                    tracing::warn!(device, "cloud live source saw no frame for 10 s");
                    break;
                }
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
        Ok(true)
    }
}

pub fn monitor_start_body(channel: u8, stream: &str, session: u32) -> serde_json::Value {
    serde_json::json!({
        "Name": "OPMonitor",
        "OPMonitor": {
            "Action": "Start",
            "Parameter": {
                "Channel": channel,
                "CombinMode": "NONE",
                "StreamType": stream,
                "TransMode": "TCP",
            },
        },
        "SessionID": format!("0x{session:08x}"),
    })
}

pub async fn start_monitor(
    connection: &mut crate::dvrip::client::Connection,
    body: &serde_json::Value,
) -> Result<(), crate::dvrip::client::DvrError> {
    connection
        .send_plain(0, crate::dvrip::msg::MONITOR_START, body)
        .await
}

pub fn live_ticket(device: &str, channel: u32, high: bool) -> Result<StreamTicket, ProviderError> {
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

fn configured_serial() -> String {
    std::env::var("PROVIEW_DEVICE_SERIAL")
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

enum DeviceCredential {
    Allowed { user: String, pass: String },
    Refused(String),
}

fn device_credential_for(
    serial: &str,
    entry: &crate::cloud::types::CloudDevice,
) -> DeviceCredential {
    let user = std::env::var("PROVIEW_DEVICE_USER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| entry.username.trim().to_string());
    let pass = std::env::var("PROVIEW_DEVICE_PASS").unwrap_or_default();
    credential_decision(&configured_serial(), serial, &user, &pass)
}

fn unready_reason(stored_token: bool, credential: &DeviceCredential) -> Option<String> {
    if stored_token {
        return None;
    }
    match credential {
        DeviceCredential::Allowed { .. } => None,
        DeviceCredential::Refused(reason) => Some(reason.clone()),
    }
}

fn credential_decision(configured: &str, serial: &str, user: &str, pass: &str) -> DeviceCredential {
    if configured.trim().is_empty() {
        return DeviceCredential::Refused(
            "no camera serial is configured, so the camera credentials are not sent to any device: set PROVIEW_DEVICE_SERIAL under Settings, Local network".to_string(),
        );
    }
    if configured.trim() != serial.trim() {
        return DeviceCredential::Refused(format!(
            "camera credentials belong to {}, not to {serial}",
            configured.trim()
        ));
    }
    if user.is_empty() || pass.is_empty() {
        return DeviceCredential::Refused(format!(
            "no camera user and password for {serial}: set PROVIEW_DEVICE_USER and PROVIEW_DEVICE_PASS"
        ));
    }
    DeviceCredential::Allowed {
        user: user.to_string(),
        pass: pass.to_string(),
    }
}

fn parameter_sets_of(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut vps: Option<Vec<u8>> = None;
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let mut index = 0usize;
    while let Some(found) = next_start_code(bytes, index) {
        let start = found + 3;
        let Some(next) = next_start_code(bytes, start) else {
            break;
        };
        let nal = &bytes[start..next];
        match crate::media::hevc::nal_type(nal) {
            Some(32) if vps.is_none() => vps = Some(nal.to_vec()),
            Some(33) if sps.is_none() => sps = Some(nal.to_vec()),
            Some(34) if pps.is_none() => pps = Some(nal.to_vec()),
            _ => {}
        }
        index = start;
    }
    match (vps, sps, pps) {
        (Some(vps), Some(sps), Some(pps)) => vec![vps, sps, pps],
        _ => Vec::new(),
    }
}

fn next_start_code(bytes: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while index + 3 <= bytes.len() {
        if bytes[index] == 0 && bytes[index + 1] == 0 && bytes[index + 2] == 1 {
            return Some(index);
        }
        index += 1;
    }
    None
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
        let envelope = self.envelope();
        if !envelope.identity().is_complete() {
            return Err(ProviderError::Misconfigured(
                MISSING_CLOUD_IDENTITY.to_string(),
            ));
        }
        let tokens = envelope
            .login_envelope(user, pass)
            .await
            .map_err(map_cloud)?;
        Ok(VendorSession {
            account: user.to_string(),
            token: Some(tokens.access),
        })
    }

    async fn devices(&self, session: &VendorSession) -> Result<Vec<Device>, ProviderError> {
        let devices = self.fetch_entries(session).await?;
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
        let Some(entry) = self.entry_of(session, device).await? else {
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
            .query(&entry.uuid, &status::auth_code_of(&entry))
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
        let ticket = live_ticket(device, channel, high)?;
        let Some(entry) = self.entry_of(session, device).await? else {
            return Err(ProviderError::UnknownDevice(format!(
                "{device} is not in this account"
            )));
        };
        let credential = device_credential_for(device.trim(), &entry);
        let stored = self.tokens.get(device.trim()).is_some();
        if let Some(reason) = unready_reason(stored, &credential) {
            return Err(ProviderError::Misconfigured(reason));
        }
        Ok(ticket)
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
    fn the_ticket_points_at_the_live_socket_and_names_the_cloud() {
        let ticket = live_ticket("7f92970ddf5282743hrc", 1, true).expect("ticket");
        assert!(matches!(ticket.kind, StreamKind::Ws));
        assert_eq!(ticket.source, ProviderSource::Cloud);
        assert_eq!(
            ticket.url,
            "/ws/live?device=7f92970ddf5282743hrc&channel=1&quality=main"
        );
        let sub = live_ticket("7f92970ddf5282743hrc", 1, false).expect("ticket");
        assert_eq!(
            sub.url,
            "/ws/live?device=7f92970ddf5282743hrc&channel=1&quality=sub"
        );
    }

    #[test]
    fn a_path_like_device_is_refused_before_a_ticket() {
        assert!(matches!(
            live_ticket("../etc/passwd", 1, true),
            Err(ProviderError::UnknownDevice(_))
        ));
        assert!(matches!(
            live_ticket("  ", 1, true),
            Err(ProviderError::UnknownDevice(_))
        ));
    }

    #[tokio::test]
    async fn the_endpoint_still_walks_the_path_before_it_issues_a_ticket() {
        let mut cfg = crate::config::Config::from_env();
        cfg.cloud_base = "http://127.0.0.1:1".to_string();
        let provider = CloudProvider::with_identity(&cfg, Identity::new("uuid", "key", "secret"));
        let session = VendorSession {
            account: "probe".to_string(),
            token: Some("access".to_string()),
        };
        let error = provider
            .stream(&session, "7f92970ddf5282743hrc", 1, true)
            .await
            .expect_err("an unreachable cloud base is not a ticket");
        assert!(
            matches!(
                error,
                ProviderError::Unreachable(_) | ProviderError::Upstream(_)
            ),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("msdlistCode") || error.to_string().contains("127.0.0.1:1")
        );
    }

    #[test]
    fn the_credentials_only_answer_for_the_configured_serial() {
        assert!(matches!(
            credential_decision("serial-one", "serial-one", "admin", "pass"),
            DeviceCredential::Allowed { .. }
        ));
        assert!(matches!(
            credential_decision("serial-one", "serial-two", "admin", "pass"),
            DeviceCredential::Refused(_)
        ));
        assert!(matches!(
            credential_decision("", "serial-one", "admin", "pass"),
            DeviceCredential::Refused(_)
        ));
    }

    #[test]
    fn a_missing_serial_refusal_names_the_setting_to_fill() {
        match credential_decision("", "serial-one", "admin", "pass") {
            DeviceCredential::Refused(reason) => {
                assert!(reason.contains("PROVIEW_DEVICE_SERIAL"), "{reason}")
            }
            DeviceCredential::Allowed { .. } => panic!("an empty serial must refuse"),
        }
    }

    #[test]
    fn an_empty_password_refuses_even_for_the_right_serial() {
        assert!(matches!(
            credential_decision("serial-one", "serial-one", "admin", ""),
            DeviceCredential::Refused(_)
        ));
    }

    #[test]
    fn a_ticket_is_refused_when_the_device_can_never_log_in() {
        let refused = credential_decision("", "serial-one", "admin", "pass");
        let reason = unready_reason(false, &refused).expect("a refusal without a token");
        assert!(reason.contains("PROVIEW_DEVICE_SERIAL"), "{reason}");
        assert!(
            unready_reason(true, &refused).is_none(),
            "a stored token means the password is not needed"
        );
        let allowed = credential_decision("serial-one", "serial-one", "admin", "pass");
        assert!(unready_reason(false, &allowed).is_none());
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

#[cfg(test)]
mod stream_tests {
    use super::*;

    fn media_body() -> Vec<u8> {
        let mut body = vec![0u8; 16];
        for nal in [
            vec![0x40, 0x01, 0x0c, 0x01],
            vec![0x42, 0x01, 0x01, 0x60],
            vec![0x44, 0x01, 0xc0],
            vec![0x26, 0x01, 0xaf],
        ] {
            body.extend_from_slice(&[0, 0, 0, 1]);
            body.extend_from_slice(&nal);
        }
        body
    }

    #[tokio::test]
    async fn the_device_streams_once_the_start_arrives_as_msgid_1410() {
        let (address, peer) = crate::dvrip::fake::serve_scripted(|_, id, _| {
            if id == crate::dvrip::msg::MONITOR_CHANNEL as u16 {
                return Vec::new();
            }
            if id == 0x0585 {
                return vec![(0x0586, b"{\"Ret\":100}".to_vec())];
            }
            if id == crate::dvrip::msg::MONITOR_START {
                return vec![(crate::media::live::LIVE_MEDIA_ID, media_body())];
            }
            Vec::new()
        })
        .await;
        let mut connection = crate::dvrip::fake::connect(&address).await;
        let body = monitor_start_body(0, "Main", connection.session());
        start_monitor(&mut connection, &body).await.expect("start");
        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), connection.recv())
            .await
            .expect("a frame inside the deadline")
            .expect("a readable frame");
        assert_eq!(frame.0.id, crate::media::live::LIVE_MEDIA_ID);
        assert_eq!(frame.1, media_body());
        drop(connection);
        let seen = peer.await.expect("peer task");
        assert_eq!(seen.last().map(|row| row.1), Some(0x0582));
    }

    #[test]
    fn parameter_sets_need_one_of_each_type() {
        let mut body = Vec::new();
        for (nal, kind) in [
            (vec![0x40u8, 0x01, 0x0c], 32u8),
            (vec![0x42, 0x01, 0x01], 33),
        ] {
            body.extend_from_slice(&[0, 0, 0, 1]);
            body.extend_from_slice(&nal);
            assert_eq!(crate::media::hevc::nal_type(&nal), Some(kind));
        }
        assert!(
            parameter_sets_of(&body).is_empty(),
            "two of three is not a set"
        );
        body.extend_from_slice(&[0, 0, 0, 1]);
        body.extend_from_slice(&[0x44, 0x01, 0xc0]);
        body.extend_from_slice(&[0, 0, 0, 1]);
        body.extend_from_slice(&[0x26, 0x01, 0xaf]);
        let sets = parameter_sets_of(&body);
        assert_eq!(sets.len(), 3);
        assert_eq!(crate::media::hevc::nal_type(&sets[0]), Some(32));
        assert_eq!(crate::media::hevc::nal_type(&sets[1]), Some(33));
        assert_eq!(crate::media::hevc::nal_type(&sets[2]), Some(34));
    }

    #[test]
    fn a_trailing_nal_without_a_following_start_code_is_not_a_set() {
        let mut body = Vec::new();
        for nal in [
            vec![0x40u8, 0x01, 0x0c],
            vec![0x42, 0x01, 0x01],
            vec![0x44, 0x01, 0xc0],
        ] {
            body.extend_from_slice(&[0, 0, 0, 1]);
            body.extend_from_slice(&nal);
        }
        assert!(
            parameter_sets_of(&body).is_empty(),
            "a set that never closes is not a set"
        );
        body.extend_from_slice(&[0, 0, 0, 1]);
        body.extend_from_slice(&[0x26, 0x01, 0xaf]);
        let complete = parameter_sets_of(&body);
        assert_eq!(complete.len(), 3);
    }
}

#[cfg(test)]
mod entry_cache_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Stub {
        base: String,
        rows: Arc<Mutex<Vec<String>>>,
        hits: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Stub {
        fn start(rows: &[&str]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("stub listener");
            let address = listener.local_addr().expect("stub address");
            listener.set_nonblocking(true).expect("stub non blocking");
            let rows: Arc<Mutex<Vec<String>>> =
                Arc::new(Mutex::new(rows.iter().map(|row| row.to_string()).collect()));
            let hits = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let counted = hits.clone();
            let ending = stop.clone();
            let listing = rows.clone();
            let handle = std::thread::spawn(move || {
                while !ending.load(Ordering::Relaxed) {
                    let (stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                    };
                    serve(stream, &listing, &counted);
                }
            });
            Self {
                base: format!("http://{address}"),
                rows,
                hits,
                stop,
                handle: Some(handle),
            }
        }

        fn set_rows(&self, rows: &[&str]) {
            let mut held = self.rows.lock().expect("rows");
            *held = rows.iter().map(|row| row.to_string()).collect();
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::Relaxed)
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                handle.join().ok();
            }
        }
    }

    fn serve(stream: TcpStream, rows: &Mutex<Vec<String>>, hits: &AtomicUsize) {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let Ok(mut writer) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        if length > 0 && reader.read_exact(&mut body).is_err() {
            return;
        }
        if request_line.contains("msdlistCode") {
            hits.fetch_add(1, Ordering::Relaxed);
        }
        let serials = rows.lock().expect("rows").clone();
        let entries: Vec<String> = serials
            .iter()
            .map(|serial| format!("{{\"uuid\":\"{serial}\",\"nickname\":\"camera\"}}"))
            .collect();
        let payload = format!(
            "{{\"code\":2000,\"msg\":\"SUCCESS\",\"data\":{{\"mine\":[{}]}}}}",
            entries.join(",")
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        let _ = writer.write_all(response.as_bytes());
    }

    fn provider(stub: &Stub) -> CloudProvider {
        let mut cfg = crate::config::Config::from_env();
        cfg.cloud_base = stub.base.clone();
        CloudProvider::with_identity(&cfg, Identity::new("uuid", "key", "secret"))
    }

    fn session(account: &str, token: &str) -> VendorSession {
        VendorSession {
            account: account.to_string(),
            token: Some(token.to_string()),
        }
    }

    #[tokio::test]
    async fn one_session_asks_for_the_device_list_once_for_every_serial() {
        let stub = Stub::start(&["serial-one", "serial-two"]);
        let provider = provider(&stub);
        let session = session("probe", "access");

        assert!(provider.owns(&session, "serial-one").await.expect("first"));
        assert!(provider.owns(&session, "serial-one").await.expect("second"));
        assert!(provider.owns(&session, "serial-two").await.expect("third"));
        assert_eq!(
            stub.hits(),
            1,
            "one session must pay one device list for every serial it owns"
        );
    }

    #[tokio::test]
    async fn another_session_pays_its_own_device_list() {
        let stub = Stub::start(&["serial-one"]);
        let provider = provider(&stub);

        assert!(provider
            .owns(&session("probe", "access-one"), "serial-one")
            .await
            .expect("first session"));
        assert!(provider
            .owns(&session("probe", "access-two"), "serial-one")
            .await
            .expect("second session"));
        assert_eq!(stub.hits(), 2, "each session keys its own entries");
    }

    #[tokio::test]
    async fn an_entry_older_than_the_window_is_fetched_again_for_a_serial_it_owns() {
        let stub = Stub::start(&["serial-one"]);
        let provider = provider(&stub).with_entry_ttl(Duration::from_millis(50));
        let session = session("probe", "access");

        assert!(provider.owns(&session, "serial-one").await.expect("first"));
        std::thread::sleep(Duration::from_millis(120));
        assert!(provider.owns(&session, "serial-one").await.expect("second"));
        assert_eq!(stub.hits(), 2, "a closed window asks the cloud again");
    }

    #[tokio::test]
    async fn a_serial_the_window_does_not_carry_is_asked_again() {
        let stub = Stub::start(&["serial-one"]);
        let provider = provider(&stub);
        let session = session("probe", "access");

        assert!(provider.owns(&session, "serial-one").await.expect("first"));
        stub.set_rows(&["serial-one", "serial-two"]);
        assert!(
            provider
                .owns(&session, "serial-two")
                .await
                .expect("new one"),
            "a camera bound inside the window must open"
        );
        assert_eq!(
            stub.hits(),
            2,
            "a serial the cached list does not carry is a miss, not a refusal"
        );
    }

    #[tokio::test]
    async fn a_fresh_fetch_replaces_the_window_for_every_serial() {
        let stub = Stub::start(&["serial-one"]);
        let provider = provider(&stub);
        let session = session("probe", "access");

        assert!(provider.owns(&session, "serial-one").await.expect("first"));
        stub.set_rows(&["serial-one", "serial-two"]);
        provider.fetch_entries(&session).await.expect("refresh");
        assert!(
            provider
                .owns(&session, "serial-two")
                .await
                .expect("new one"),
            "the refreshed list must answer without another call"
        );
        assert_eq!(
            stub.hits(),
            2,
            "the refresh is the second call, the lookup is free"
        );
    }

    #[tokio::test]
    async fn a_serial_removed_by_a_refresh_stops_answering() {
        let stub = Stub::start(&["serial-one", "serial-two"]);
        let provider = provider(&stub);
        let session = session("probe", "access");

        assert!(provider.owns(&session, "serial-one").await.expect("first"));
        stub.set_rows(&["serial-two"]);
        provider.fetch_entries(&session).await.expect("refresh");
        assert!(
            !provider
                .owns(&session, "serial-one")
                .await
                .expect("unbound"),
            "a camera the account no longer lists must stop opening"
        );
    }
}

#[cfg(test)]
mod login_tests {
    use super::*;
    use crate::dvrip::fake::{connect, json_reply, serve_scripted};
    use crate::dvrip::frame::Header;
    use crate::dvrip::msg;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn provider() -> CloudProvider {
        let cfg = Config::from_env();
        CloudProvider::with_identity(&cfg, Identity::new("test-uuid", "test-key", "test-secret"))
    }

    #[tokio::test]
    async fn token_login_ignores_a_reply_carrying_another_message_id() {
        let (address, peer) = serve_scripted(|_, id, _| {
            if id == msg::LOGIN {
                return vec![
                    (
                        MONITOR_CLAIM_REPLY,
                        json_reply(serde_json::json!({ "Ret": 100 })),
                    ),
                    (
                        msg::LOGIN_REPLY,
                        json_reply(serde_json::json!({ "Ret": 999 })),
                    ),
                ];
            }
            Vec::new()
        })
        .await;
        let mut connection = connect(&address).await;
        let control = crate::media::dhav::MediaCipher::channel();
        let ret = provider()
            .token_login(&mut connection, "stale-token", &control)
            .await;
        assert_eq!(ret, Ok(999));
        drop(connection);
        peer.await.expect("peer task");
    }

    #[tokio::test]
    async fn the_password_login_skips_a_late_claim_reply() {
        let (address, peer) = serve_scripted(|_, id, _| {
            if id == msg::LOGIN {
                return vec![
                    (
                        MONITOR_CLAIM_REPLY,
                        json_reply(serde_json::json!({ "Ret": 100 })),
                    ),
                    (
                        msg::LOGIN_REPLY,
                        json_reply(serde_json::json!({
                            "Ret": 100,
                            "AdminToken": "REAL",
                            "SessionID": "0x22222222"
                        })),
                    ),
                ];
            }
            Vec::new()
        })
        .await;
        let mut connection = connect(&address).await;
        let reply = connection
            .login_reply("admin", "pass")
            .await
            .expect("the password login reads its own reply");
        assert_eq!(reply["AdminToken"].as_str(), Some("REAL"));
        assert_eq!(connection.session(), 0x2222_2222);
        drop(connection);
        peer.await.expect("peer task");
    }

    const SERIAL: &str = "zha4183-tunnel-reopen";

    fn credential_env() {
        std::env::set_var("PROVIEW_DEVICE_SERIAL", SERIAL);
        std::env::set_var("PROVIEW_DEVICE_USER", "admin");
        std::env::set_var("PROVIEW_DEVICE_PASS", "secret");
    }

    fn entry(serial: &str) -> crate::cloud::types::CloudDevice {
        crate::cloud::types::CloudDevice {
            uuid: serial.to_string(),
            nickname: String::new(),
            username: "admin".to_string(),
            code: None,
            model: None,
            channel: None,
            channel_num: None,
            extra: serde_json::Map::new(),
        }
    }

    fn provider_with_store(tag: &str) -> (CloudProvider, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("zha4183-tokens-{tag}"));
        std::fs::create_dir_all(&dir).expect("token dir");
        let mut provider = provider();
        provider.tokens = crate::cloud::DeviceTokenStore::load(Some(&dir));
        (provider, dir)
    }

    struct LoginPeer {
        address: String,
        events: Arc<Mutex<Vec<String>>>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for LoginPeer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    impl LoginPeer {
        async fn start(token_ret: i64, late: Duration) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("peer listener");
            let address = listener.local_addr().expect("peer address").to_string();
            let events = Arc::new(Mutex::new(Vec::<String>::new()));
            let shared = events.clone();
            let handle = tokio::spawn(async move {
                let mut index = 0usize;
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        return;
                    };
                    index += 1;
                    let events = shared.clone();
                    tokio::spawn(async move {
                        serve_peer(socket, index, token_ret, late, events).await;
                    });
                }
            });
            Self {
                address,
                events,
                handle,
            }
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("peer events").clone()
        }
    }

    fn note(events: &Arc<Mutex<Vec<String>>>, text: String) {
        events.lock().expect("peer events").push(text);
    }

    async fn send_reply(
        socket: &mut tokio::net::TcpStream,
        request: &Header,
        session: u32,
        id: u16,
        payload: Vec<u8>,
    ) -> std::io::Result<()> {
        let header = Header {
            session,
            sequence: request.sequence,
            channel: request.channel,
            end: false,
            id,
            length: payload.len() as u32,
        };
        socket.write_all(&header.encode()).await?;
        socket.write_all(&payload).await
    }

    async fn serve_peer(
        mut socket: tokio::net::TcpStream,
        index: usize,
        token_ret: i64,
        late: Duration,
        events: Arc<Mutex<Vec<String>>>,
    ) {
        let claim_reply = json_reply(serde_json::json!({ "Ret": 100 }));
        loop {
            let mut header_bytes = [0u8; crate::dvrip::HEADER_LEN];
            if socket.read_exact(&mut header_bytes).await.is_err() {
                return;
            }
            let Ok(request) = Header::decode(&header_bytes) else {
                return;
            };
            let mut body = vec![0u8; request.length as usize];
            if socket.read_exact(&mut body).await.is_err() {
                return;
            }
            let token_channel = request.channel == msg::MONITOR_CHANNEL;
            let label = match (token_channel, request.id) {
                (true, msg::MONITOR) => "pre_claim",
                (true, msg::LOGIN) => "token_login",
                (false, msg::LOGIN) => "password_login",
                _ => "other",
            };
            note(&events, format!("conn{index}:{label}"));
            if token_channel && request.id == msg::MONITOR {
                if send_reply(
                    &mut socket,
                    &request,
                    request.session,
                    MONITOR_CLAIM_REPLY,
                    claim_reply.clone(),
                )
                .await
                .is_err()
                {
                    return;
                }
                continue;
            }
            if token_channel && request.id == msg::LOGIN {
                if token_ret == 100 {
                    let payload = json_reply(serde_json::json!({
                        "Ret": 100,
                        "SessionID": "0x33333333"
                    }));
                    if send_reply(
                        &mut socket,
                        &request,
                        0x3333_3333,
                        msg::LOGIN_REPLY,
                        payload,
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                    continue;
                }
                let payload = json_reply(serde_json::json!({ "Ret": token_ret }));
                if send_reply(
                    &mut socket,
                    &request,
                    request.session,
                    msg::LOGIN_REPLY,
                    payload,
                )
                .await
                .is_err()
                {
                    return;
                }
                tokio::time::sleep(late).await;
                let payload = json_reply(serde_json::json!({
                    "Ret": 100,
                    "AdminToken": "STALE",
                    "SessionID": "0x11111111"
                }));
                let reply = Header {
                    session: 0x1111_1111,
                    sequence: 1,
                    channel: 0,
                    end: false,
                    id: msg::LOGIN_REPLY,
                    length: payload.len() as u32,
                };
                if socket.write_all(&reply.encode()).await.is_ok()
                    && socket.write_all(&payload).await.is_ok()
                {
                    note(&events, "late_token_reply_written".to_string());
                } else {
                    note(&events, "late_token_reply_rejected".to_string());
                }
                continue;
            }
            if !token_channel && request.id == msg::LOGIN {
                let payload = json_reply(serde_json::json!({
                    "Ret": 100,
                    "AdminToken": "REAL",
                    "SessionID": "0x22222222"
                }));
                if send_reply(
                    &mut socket,
                    &request,
                    0x2222_2222,
                    msg::LOGIN_REPLY,
                    payload,
                )
                .await
                .is_err()
                {
                    return;
                }
            }
        }
    }

    #[tokio::test]
    async fn device_login_reopens_the_tunnel_before_the_password_login() {
        credential_env();
        let peer = LoginPeer::start(106, Duration::from_millis(700)).await;
        let (provider, dir) = provider_with_store("reopen");
        provider
            .tokens
            .put(SERIAL, "expired-token")
            .expect("stored token");
        let reopened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reopen = {
            let address = peer.address.clone();
            let reopened = reopened.clone();
            move || {
                reopened.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let address = address.clone();
                async move { Ok::<_, ProviderError>(connect(&address).await) }
            }
        };
        let first = connect(&peer.address).await;
        let (connection, note) = provider
            .device_login(first, &entry(SERIAL), reopen)
            .await
            .expect("the password login finishes the session");
        assert_eq!(
            connection.session(),
            0x2222_2222,
            "the session must belong to the password reply, not to a late token reply"
        );
        assert!(note.contains("password login ret 100"), "{note}");
        assert!(note.contains("token dropped"), "{note}");
        assert_eq!(
            reopened.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "a stale token must cost exactly one new tunnel"
        );
        let events = peer.events();
        assert!(
            events.contains(&"conn1:token_login".to_string()),
            "{events:?}"
        );
        assert!(
            events.contains(&"conn2:password_login".to_string()),
            "the password login must run on the reopened tunnel: {events:?}"
        );
        drop(connection);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn device_login_keeps_the_tunnel_when_the_stored_token_is_accepted() {
        credential_env();
        let peer = LoginPeer::start(100, Duration::from_millis(700)).await;
        let (provider, dir) = provider_with_store("accepted");
        provider
            .tokens
            .put(SERIAL, "live-token")
            .expect("stored token");
        let reopened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reopen = {
            let reopened = reopened.clone();
            move || {
                reopened.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async move {
                    Err::<crate::dvrip::client::Connection, ProviderError>(ProviderError::Upstream(
                        "the token tunnel must be kept".to_string(),
                    ))
                }
            }
        };
        let first = connect(&peer.address).await;
        let (connection, note) = provider
            .device_login(first, &entry(SERIAL), reopen)
            .await
            .expect("the token login finishes the session");
        assert_eq!(connection.session(), 0x3333_3333);
        assert!(note.contains("stored device token ret 100"), "{note}");
        assert_eq!(reopened.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(!peer
            .events()
            .iter()
            .any(|event| event.contains("password_login")));
        drop(connection);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn device_login_without_a_stored_token_keeps_the_first_tunnel() {
        credential_env();
        let peer = LoginPeer::start(106, Duration::from_millis(700)).await;
        let (provider, dir) = provider_with_store("bare");
        let reopened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reopen = {
            let reopened = reopened.clone();
            move || {
                reopened.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async move {
                    Err::<crate::dvrip::client::Connection, ProviderError>(ProviderError::Upstream(
                        "no token means no reopen".to_string(),
                    ))
                }
            }
        };
        let first = connect(&peer.address).await;
        let (connection, note) = provider
            .device_login(first, &entry(SERIAL), reopen)
            .await
            .expect("the password login finishes the session");
        assert_eq!(connection.session(), 0x2222_2222);
        assert!(note.contains("no stored device token"), "{note}");
        assert_eq!(reopened.load(std::sync::atomic::Ordering::Relaxed), 0);
        drop(connection);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
