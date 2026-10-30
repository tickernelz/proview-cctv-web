use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

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
const TOKEN_REFUSAL_RETS: [i64; 2] = [117, 137];

use crate::config::Config;

struct SessionEntries {
    at: Instant,
    devices: Arc<Vec<CloudDevice>>,
}

struct Refusal {
    ret: i64,
    token: String,
}

struct RelayTarget {
    rps: String,
    host: String,
    port: u16,
}

type Gate = Arc<AsyncMutex<Option<(Instant, CloudError)>>>;

pub struct CloudProvider {
    http: reqwest::Client,
    base: String,
    identity: Option<Identity>,
    counter: Arc<std::sync::atomic::AtomicU32>,
    status: StatusClient,
    tokens: crate::cloud::DeviceTokenStore,
    token_refusals: Mutex<HashMap<String, Refusal>>,
    entries: Mutex<HashMap<String, SessionEntries>>,
    gates: Mutex<HashMap<String, Gate>>,
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
            token_refusals: Mutex::new(HashMap::new()),
            entries: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
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

    fn entries_since(&self, session: &str, arrived: Instant) -> Option<Arc<Vec<CloudDevice>>> {
        let mut entries = match self.entries.lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        let fresh = match entries.get(session) {
            Some(entry) => entry.at.elapsed() < self.entry_ttl,
            None => false,
        };
        if !fresh {
            entries.remove(session);
            return None;
        }
        entries
            .get(session)
            .filter(|entry| entry.at >= arrived)
            .map(|entry| entry.devices.clone())
    }

    fn gate(&self, token: &str) -> Gate {
        let mut gates = match self.gates.lock() {
            Ok(gates) => gates,
            Err(poisoned) => poisoned.into_inner(),
        };
        gates
            .entry(token.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(None)))
            .clone()
    }

    fn release_gates(&self) {
        if let Ok(mut gates) = self.gates.lock() {
            gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        }
    }

    #[cfg(test)]
    fn gate_count(&self) -> usize {
        self.gates.lock().map(|gates| gates.len()).unwrap_or(0)
    }

    async fn entries_once(
        &self,
        session: &VendorSession,
        adopt: bool,
    ) -> Result<Arc<Vec<CloudDevice>>, ProviderError> {
        let entries = self.entries_guarded(session, adopt).await;
        self.release_gates();
        entries
    }

    async fn entries_guarded(
        &self,
        session: &VendorSession,
        adopt: bool,
    ) -> Result<Arc<Vec<CloudDevice>>, ProviderError> {
        let token = self.token(session)?;
        let gate = self.gate(token);
        let arrived = Instant::now();
        let mut flight = gate.lock().await;
        if adopt {
            if let Some((at, error)) = flight.as_ref() {
                if *at >= arrived {
                    return Err(map_cloud(error.clone()));
                }
            }
            if let Some(devices) = self.entries_since(token, arrived) {
                return Ok(devices);
            }
        }
        match self.envelope().device_list(token).await {
            Ok(devices) => {
                let devices = Arc::new(devices);
                self.remember_entries(token, devices.clone());
                *flight = None;
                Ok(devices)
            }
            Err(error) => {
                *flight = Some((Instant::now(), error.clone()));
                Err(map_cloud(error))
            }
        }
    }

    async fn fetch_entries(
        &self,
        session: &VendorSession,
    ) -> Result<Arc<Vec<CloudDevice>>, ProviderError> {
        self.entries_once(session, false).await
    }

    async fn fetch_entries_shared(
        &self,
        session: &VendorSession,
    ) -> Result<Arc<Vec<CloudDevice>>, ProviderError> {
        self.entries_once(session, true).await
    }

    fn token<'a>(&self, session: &'a VendorSession) -> Result<&'a str, ProviderError> {
        session.token.as_deref().ok_or_else(|| {
            ProviderError::Misconfigured(
                "the session carries no cloud token; sign in again".to_string(),
            )
        })
    }
    fn token_refusal(&self, serial: &str) -> Option<Refusal> {
        let refusals = match self.token_refusals.lock() {
            Ok(refusals) => refusals,
            Err(poisoned) => poisoned.into_inner(),
        };
        refusals.get(serial).map(|refusal| Refusal {
            ret: refusal.ret,
            token: refusal.token.clone(),
        })
    }

    fn remember_token_refusal(&self, serial: &str, ret: i64, token: &str) -> bool {
        if !TOKEN_REFUSAL_RETS.contains(&ret) {
            return false;
        }
        let mut refusals = match self.token_refusals.lock() {
            Ok(refusals) => refusals,
            Err(poisoned) => poisoned.into_inner(),
        };
        refusals.insert(
            serial.to_string(),
            Refusal {
                ret,
                token: token.to_string(),
            },
        );
        true
    }

    fn forget_token_refusal(&self, serial: &str) {
        let mut refusals = match self.token_refusals.lock() {
            Ok(refusals) => refusals,
            Err(poisoned) => poisoned.into_inner(),
        };
        refusals.remove(serial);
    }

    async fn device_login(
        &self,
        mut connection: crate::dvrip::client::Connection,
        entry: &crate::cloud::types::CloudDevice,
        control: &crate::media::dhav::MediaCipher,
        session_key: &str,
    ) -> Result<(crate::dvrip::client::Connection, String), ProviderError> {
        let serial = entry.uuid.trim().to_string();
        let stored = self.tokens.get(&serial);
        let source = match stored {
            Some(_) => "stored device token",
            None => "account device token",
        };
        let Some(device_token) = stored.or_else(|| account_device_token(entry)) else {
            return Err(no_device_token(&serial));
        };
        if let Some(refusal) = self.token_refusal(&serial) {
            if refusal.token == device_token {
                return Err(login_refused(
                    &serial,
                    refusal.ret,
                    "device token earlier in this process",
                ));
            }
            self.forget_token_refusal(&serial);
        }
        let offered = crate::dvrip::crypto::open_login_name(&serial, &device_token)
            .unwrap_or_else(|| device_token.clone());
        match self
            .token_login(&mut connection, &offered, control, session_key)
            .await
        {
            Ok(100) => {
                if self.tokens.get(&serial).is_none() {
                    let _ = self.tokens.put(&serial, &device_token);
                }
                self.forget_token_refusal(&serial);
                let note = format!("{source} ret 100, session 0x{:08x}", connection.session());
                Ok((connection, note))
            }
            Ok(ret) => {
                let _ = self.tokens.forget(&serial);
                self.remember_token_refusal(&serial, ret, &device_token);
                Err(login_refused(&serial, ret, source))
            }
            Err(error) => {
                let _ = self.tokens.forget(&serial);
                Err(error)
            }
        }
    }

    async fn token_login(
        &self,
        connection: &mut crate::dvrip::client::Connection,
        token: &str,
        control: &crate::media::dhav::MediaCipher,
        session_key: &str,
    ) -> Result<i64, ProviderError> {
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
            .map_err(|error| login_stage_error("pre-claim", format!("{error:?}")))?;
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
                crate::dvrip::crypto::communicate_key(modulus.trim(), exponent.trim(), session_key)
                    .map_err(|error| login_stage_error("token", error))?
            }
            None => String::new(),
        };
        let login_body = serde_json::json!({
            "EncryptType": "TOKEN",
            "LoginType": "DVRIP-FutureHome",
            "UserName": token,
            "PassWord": "",
            "CommunicateKey": communicate_key,
            "User-Agent": format!("Funv1.2.2c-And-{session_key}"),
        });
        connection
            .send_encrypted(
                control,
                crate::dvrip::msg::MONITOR_CHANNEL,
                crate::dvrip::msg::LOGIN,
                &login_body,
            )
            .await
            .map_err(|error| login_stage_error("token", format!("{error:?}")))?;
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
                    if ret == 100 {
                        if let Some(session) = reply
                            .get("SessionID")
                            .and_then(serde_json::Value::as_str)
                            .and_then(crate::dvrip::client::parse_session_id)
                        {
                            connection.set_session(session);
                        }
                    }
                    return Ok(ret);
                }
            }
        }
        Err(login_stage_error(
            "token",
            "the device answered no login reply",
        ))
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
        let devices = self.fetch_entries_shared(session).await?;
        Ok(devices.iter().find(|entry| entry.uuid == serial).cloned())
    }

    pub async fn owns(&self, session: &VendorSession, device: &str) -> Result<bool, ProviderError> {
        Ok(self.entry_of(session, device).await?.is_some())
    }

    async fn relay_target(&self, entry: &CloudDevice) -> Result<RelayTarget, ProviderError> {
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
        Ok(RelayTarget { rps, host, port })
    }

    async fn dial(
        &self,
        entry: &CloudDevice,
        target: &RelayTarget,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
        let client_token = rps_client::client_token();
        let session_id = rps_client::session_id();
        let request = p2p::agent_request(
            &target.rps,
            &client_token,
            &entry.uuid,
            p2p::AGENT_SERVICE_MEDIA,
            &session_id,
            p2p::AGENT_STEP_MEDIA,
        );
        let agent = rps_client::AccessClient::new()
            .assign(&target.host, target.port, &request)
            .await
            .map_err(|error| {
                ProviderError::Upstream(format!("agent assignment failed: {error}"))
            })?;
        let frame = p2p::handshake_frame(&target.rps, &client_token, &entry.uuid, &session_id);
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

    pub async fn open_control(
        &self,
        session: &VendorSession,
        device: &str,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
        self.open_session(session, device, true).await
    }

    pub async fn open_talk(
        &self,
        session: &VendorSession,
        device: &str,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
        self.open_session(session, device, false).await
    }

    async fn open_session(
        &self,
        session: &VendorSession,
        device: &str,
        seal: bool,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
        match self.login_session(session, device, seal).await {
            Err(first) if open_is_worth_retrying(&first) => {
                tracing::warn!(device = %device, %first, seal, "cloud session failed, opening another tunnel");
                self.login_session(session, device, seal)
                    .await
                    .map_err(|second| ProviderError::Upstream(format!("{first}; retry: {second}")))
            }
            outcome => outcome,
        }
    }

    async fn login_session(
        &self,
        session: &VendorSession,
        device: &str,
        seal: bool,
    ) -> Result<crate::dvrip::client::Connection, ProviderError> {
        let Some(entry) = self.entry_of(session, device.trim()).await? else {
            return Err(ProviderError::UnknownDevice(device.trim().to_string()));
        };
        let target = self.relay_target(&entry).await?;
        let connection = self.dial(&entry, &target).await?;
        let control = crate::media::dhav::MediaCipher::channel();
        let session_key = rps_client::session_key();
        let (mut connection, note) = self
            .device_login(connection, &entry, &control, &session_key)
            .await?;
        if seal {
            let sealed =
                crate::media::dhav::MediaCipher::from_text(&session_key).ok_or_else(|| {
                    ProviderError::LoginRefused(format!(
                        "cloud login stage=session key: a {}-character key cannot seal control frames",
                        session_key.len()
                    ))
                })?;
            connection.set_seal(sealed);
        }
        tracing::debug!(device = %entry.uuid, %note, seal, "cloud device login");
        Ok(connection)
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
        let target = self.relay_target(&entry).await?;
        let connection = self.dial(&entry, &target).await?;
        let control = crate::media::dhav::MediaCipher::channel();
        let session_key = rps_client::session_key();
        let (mut connection, login_note) = self
            .device_login(connection, &entry, &control, &session_key)
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

    pub async fn log_in(
        &self,
        connection: crate::dvrip::client::Connection,
        entry: &CloudDevice,
        session_key: &str,
    ) -> Result<(crate::dvrip::client::Connection, String), ProviderError> {
        let control = crate::media::dhav::MediaCipher::channel();
        self.device_login(connection, entry, &control, session_key)
            .await
    }

    pub async fn open_playback(
        &self,
        session: &VendorSession,
        device: &str,
    ) -> Result<(crate::dvrip::playback::Session, String), ProviderError> {
        let session_key = rps_client::session_key();
        self.open_playback_with_key(session, device, &session_key)
            .await
    }

    pub async fn open_playback_with_key(
        &self,
        session: &VendorSession,
        device: &str,
        session_key: &str,
    ) -> Result<(crate::dvrip::playback::Session, String), ProviderError> {
        let identity = self.identity.clone().unwrap_or_else(Identity::resolve);
        if !identity.is_complete() {
            return Err(ProviderError::Misconfigured(
                MISSING_CLOUD_IDENTITY.to_string(),
            ));
        }
        let Some(entry) = self.entry_of(session, device.trim()).await? else {
            return Err(ProviderError::UnknownDevice(device.trim().to_string()));
        };
        let target = self.relay_target(&entry).await?;
        let connection = self.dial(&entry, &target).await?;
        let control = crate::media::dhav::MediaCipher::channel();
        let (connection, note) = self
            .device_login(connection, &entry, &control, session_key)
            .await?;
        let sealed = crate::media::dhav::MediaCipher::from_text(session_key).ok_or_else(|| {
            ProviderError::LoginRefused(format!(
                "cloud login stage=session key: a {}-character key cannot seal control frames",
                session_key.len()
            ))
        })?;
        Ok((
            crate::dvrip::playback::Session::from_connection(connection, sealed),
            note,
        ))
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
        let mut meta = crate::live::StreamMeta {
            video_codec: Some("H265".to_string()),
            ..crate::live::StreamMeta::default()
        };
        hub.set_meta(&key, meta.clone());
        let mut demux = crate::media::live::LiveDemux::new();
        let mut primed: Vec<crate::media::live::LiveFrame> = Vec::new();
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
            let frames = demux.push_frames(&payload);
            if frames.is_empty() {
                continue;
            }
            for frame in frames {
                buffered.extend_from_slice(&frame.bytes);
                primed.push(frame);
            }
            parameter_sets = parameter_sets_of(&buffered);
        }
        if parameter_sets.len() < 3 {
            return Err(ProviderError::Upstream(
                "the cloud stream did not carry VPS, SPS and PPS within 10 s".to_string(),
            ));
        }
        match key.mode {
            crate::live::LiveMode::Passthrough if passthrough_fits(&parameter_sets) => {
                let config = crate::media::mux::VideoConfig::hevc(
                    &parameter_sets[0],
                    &parameter_sets[1],
                    &parameter_sets[2],
                )
                .map_err(|error| {
                    ProviderError::Upstream(format!("the cloud stream SPS did not parse: {error}"))
                })?;
                let _sender = hub.publisher(key.clone());
                crate::source::publish_config(&hub, &key, &mut meta, &config);
                let mut relay = PassthroughRelay::new(config);
                for frame in primed {
                    relay.frame(&hub, &key, frame);
                }
                loop {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let frame =
                        tokio::time::timeout(std::time::Duration::from_secs(10), connection.recv())
                            .await;
                    match frame {
                        Ok(Ok((header, payload))) => {
                            if header.id != crate::media::live::LIVE_MEDIA_ID {
                                continue;
                            }
                            for frame in demux.push_frames(&payload) {
                                relay.frame(&hub, &key, frame);
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
                relay.finish(&hub, &key);
                let stats = demux.stats();
                tracing::info!(
                    device,
                    fragments = relay.fragments,
                    keyframes = stats.keyframes,
                    deltas = stats.deltas,
                    dropped = stats.dropped,
                    fps = relay.rate.per_second(),
                    "cloud passthrough source closed"
                );
                Ok(true)
            }
            _ => {
                self.relay_transcode(
                    &hub,
                    &key,
                    connection,
                    demux,
                    buffered,
                    &parameter_sets,
                    stop,
                    device,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn relay_transcode(
        &self,
        hub: &std::sync::Arc<crate::live::LiveHub>,
        key: &crate::live::StreamKey,
        mut connection: crate::dvrip::client::Connection,
        mut demux: crate::media::live::LiveDemux,
        mut buffered: Vec<u8>,
        parameter_sets: &[Vec<u8>],
        stop: &std::sync::atomic::AtomicBool,
        device: &str,
    ) -> Result<bool, ProviderError> {
        let (mut stdin, reader, child) =
            crate::source::spawn_fmp4_publisher(hub, key, parameter_sets)
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

fn passthrough_fits(sets: &[Vec<u8>]) -> bool {
    matches!(sets, [vps, sps, pps] if crate::media::mux::VideoConfig::hevc(vps, sps, pps)
        .is_ok_and(|config| !config.reorders))
}

struct PassthroughRelay {
    mux: crate::source::HevcPassthrough,
    rate: crate::media::mux::LiveRate,
    ticks: crate::media::mux::Ticks,
    fragments: u64,
}

impl PassthroughRelay {
    fn new(config: crate::media::mux::VideoConfig) -> Self {
        Self {
            mux: crate::source::HevcPassthrough::with_config(config),
            rate: crate::media::mux::LiveRate::new(),
            ticks: crate::media::mux::Ticks::default(),
            fragments: 0,
        }
    }

    fn frame(
        &mut self,
        hub: &crate::live::LiveHub,
        key: &crate::live::StreamKey,
        frame: crate::media::live::LiveFrame,
    ) {
        self.rate.observe(frame.keyframe, frame.stamp, frame.rate);
        let nals = crate::media::mux::split_annexb(&frame.bytes);
        let duration = self.ticks.next(self.rate.per_second());
        for (fragment, keyframe) in self.mux.push(&nals, frame.keyframe, duration) {
            self.fragments += 1;
            hub.publish_media(key, fragment, keyframe);
        }
    }

    fn finish(&mut self, hub: &crate::live::LiveHub, key: &crate::live::StreamKey) {
        if let Some((fragment, keyframe)) = self.mux.flush() {
            self.fragments += 1;
            hub.publish_media(key, fragment, keyframe);
        }
    }
}

#[cfg(test)]
mod passthrough_tests {
    use super::PassthroughRelay;
    use crate::live::{LiveHub, LiveMode, PacketKind, StreamKey, StreamMeta};
    use crate::media::live::LiveFrame;
    use crate::media::mux::VideoConfig;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    fn sets() -> [Vec<u8>; 3] {
        [
            hex("40010c01ffff0160000003000003000003000003003f2c0c00000fa00000bb8140"),
            hex("4201010160000003000003000003000003003fa001202002887f9cbabc12e080"),
            hex("4401c0f3c0e14524"),
        ]
    }

    fn frame(keyframe: bool) -> LiveFrame {
        let mut bytes = Vec::new();
        if keyframe {
            for set in sets() {
                bytes.extend_from_slice(&[0, 0, 0, 1]);
                bytes.extend_from_slice(&set);
            }
        }
        bytes.extend_from_slice(&[0, 0, 0, 1, if keyframe { 0x26 } else { 0x02 }, 0x01, 0xaf]);
        LiveFrame {
            keyframe,
            rate: keyframe.then_some(12),
            stamp: None,
            bytes,
        }
    }

    #[test]
    fn only_a_stream_without_reordering_is_passed_through() {
        assert!(super::passthrough_fits(&sets()));
        let b_frames = [
            hex("40010c01ffff01600000030090000003000003003c959809"),
            hex("42010101600000030090000003000003003ca00a080f165959a4932bc05a020000030002000003003210"),
            hex("4401c172b46240"),
        ];
        assert!(!super::passthrough_fits(&b_frames));
        assert!(!super::passthrough_fits(&sets()[..2]));
    }

    #[tokio::test]
    async fn cloud_passthrough_publishes_hvc1_init_then_one_fragment_per_frame() {
        let hub = LiveHub::new(8);
        let key = StreamKey::cloud("account", "serial-one", 1, false, LiveMode::Passthrough);
        let mut subscription = hub.subscribe(key.clone());
        let [vps, sps, pps] = sets();
        let config = VideoConfig::hevc(&vps, &sps, &pps).expect("config");
        let mut meta = StreamMeta::default();
        crate::source::publish_config(&hub, &key, &mut meta, &config);
        let init = subscription
            .init
            .lock()
            .expect("init")
            .clone()
            .expect("an init");
        assert!(init.windows(4).any(|window| window == b"hvc1"));
        let published = hub.meta(&key).expect("meta");
        assert_eq!(published.codec_string.as_deref(), Some("hvc1.1.6.L63"));
        assert_eq!(
            (published.width, published.height),
            (Some(2304), Some(2592))
        );
        let mut relay = PassthroughRelay::new(config);
        relay.frame(&hub, &key, frame(false));
        relay.frame(&hub, &key, frame(true));
        relay.frame(&hub, &key, frame(false));
        relay.finish(&hub, &key);
        assert_eq!(relay.fragments, 2);
        let first = subscription
            .receiver
            .recv()
            .await
            .expect("keyframe fragment");
        assert_eq!(first.kind, PacketKind::Fragment);
        assert!(first.keyframe);
        assert!(first.bytes.windows(4).any(|window| window == b"moof"));
        assert!(
            !first
                .bytes
                .windows(sps.len())
                .any(|window| window == sps.as_slice()),
            "parameter sets live in the hvcC, not in the samples"
        );
        let second = subscription.receiver.recv().await.expect("delta fragment");
        assert!(!second.keyframe);
        hub.release(&key);
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

fn login_stage_error(stage: &str, detail: impl std::fmt::Display) -> ProviderError {
    ProviderError::LoginRefused(format!("cloud login stage={stage}: {detail}"))
}

fn login_refused(serial: &str, ret: i64, source: &str) -> ProviderError {
    ProviderError::LoginRefused(format!(
        "cloud login stage=token ret {ret} ({}): the device refused the {source} for {serial}",
        crate::dvrip::client::describe_ret(ret)
    ))
}

fn no_device_token(serial: &str) -> ProviderError {
    ProviderError::Misconfigured(format!(
        "cloud login stage=token: the account carries no device token for {serial}, and the cloud path logs in with the account token only; sign in again so the account issues one"
    ))
}

fn account_device_token(entry: &crate::cloud::types::CloudDevice) -> Option<String> {
    entry
        .extra
        .get("deviceToken")
        .and_then(|token| token.get("AdminToken"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn open_is_worth_retrying(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Upstream(_) | ProviderError::Unreachable(_) | ProviderError::Io(_)
    )
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
        let Some(candidate) = self
            .tokens
            .get(device.trim())
            .or_else(|| account_device_token(&entry))
        else {
            return Err(no_device_token(device.trim()));
        };
        if let Some(refusal) = self.token_refusal(device.trim()) {
            if refusal.token == candidate {
                return Err(login_refused(
                    device.trim(),
                    refusal.ret,
                    "device token earlier in this process",
                ));
            }
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
    fn a_log_in_that_lacks_a_token_names_the_stage_and_the_account() {
        let missing = no_device_token("serial-one").to_string();
        assert!(missing.contains("stage=token"), "{missing}");
        assert!(
            missing.contains("account carries no device token"),
            "{missing}"
        );
        assert!(!missing.contains("PROVIEW_DEVICE"), "{missing}");
        let refused = login_refused("serial-one", 205, "account device token").to_string();
        assert!(refused.contains("stage=token"), "{refused}");
        assert!(refused.contains("ret 205"), "{refused}");
        assert!(refused.contains("ip locked"), "{refused}");
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
        status: Arc<Mutex<u16>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Stub {
        fn start(rows: &[&str]) -> Self {
            Self::build(rows, 200, Duration::ZERO)
        }

        fn with_delay(rows: &[&str], delay: Duration) -> Self {
            Self::build(rows, 200, delay)
        }

        fn failing(rows: &[&str], status: u16, delay: Duration) -> Self {
            Self::build(rows, status, delay)
        }

        fn build(rows: &[&str], status: u16, delay: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("stub listener");
            let address = listener.local_addr().expect("stub address");
            listener.set_nonblocking(true).expect("stub non blocking");
            let rows: Arc<Mutex<Vec<String>>> =
                Arc::new(Mutex::new(rows.iter().map(|row| row.to_string()).collect()));
            let hits = Arc::new(AtomicUsize::new(0));
            let reply = Arc::new(Mutex::new(status));
            let stop = Arc::new(AtomicBool::new(false));
            let counted = hits.clone();
            let ending = stop.clone();
            let listing = rows.clone();
            let answering = reply.clone();
            let handle = std::thread::spawn(move || {
                while !ending.load(Ordering::Relaxed) {
                    let (stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                    };
                    let listing = listing.clone();
                    let counted = counted.clone();
                    let answering = answering.clone();
                    std::thread::spawn(move || {
                        serve(stream, &listing, &counted, &answering, delay)
                    });
                }
            });
            Self {
                base: format!("http://{address}"),
                rows,
                hits,
                status: reply,
                stop,
                handle: Some(handle),
            }
        }

        fn set_rows(&self, rows: &[&str]) {
            let mut held = self.rows.lock().expect("rows");
            *held = rows.iter().map(|row| row.to_string()).collect();
        }

        fn set_status(&self, status: u16) {
            let mut held = self.status.lock().expect("status");
            *held = status;
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

    fn serve(
        stream: TcpStream,
        rows: &Mutex<Vec<String>>,
        hits: &AtomicUsize,
        status: &Mutex<u16>,
        delay: Duration,
    ) {
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
        let serials = rows.lock().expect("rows").clone();
        if request_line.contains("msdlistCode") {
            hits.fetch_add(1, Ordering::Relaxed);
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        }
        let status = *status.lock().expect("status");
        if status != 200 {
            let response = format!(
                "HTTP/1.1 {status} Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = writer.write_all(response.as_bytes());
            return;
        }
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parallel_cold_opens_on_one_session_share_a_single_device_list() {
        let stub = Stub::with_delay(
            &["serial-one", "serial-two", "serial-three"],
            Duration::from_millis(200),
        );
        let provider = Arc::new(provider(&stub));
        let held = Arc::new(session("probe", "access"));
        let mut pending = tokio::task::JoinSet::new();

        for serial in ["serial-one", "serial-two", "serial-three"] {
            let provider = Arc::clone(&provider);
            let held = Arc::clone(&held);
            pending.spawn(async move { provider.owns(&held, serial).await });
        }
        while let Some(joined) = pending.join_next().await {
            assert!(
                joined.expect("joined").expect("owns"),
                "every cold open must resolve against the one device list"
            );
        }
        assert_eq!(
            stub.hits(),
            1,
            "three cold opens racing on one session must ask the cloud once"
        );
    }

    #[tokio::test]
    async fn the_gate_map_does_not_grow_with_every_session() {
        let stub = Stub::start(&["serial-one"]);
        let provider = provider(&stub);

        for token in ["access-one", "access-two", "access-three"] {
            assert!(provider
                .owns(&session("probe", token), "serial-one")
                .await
                .expect(token));
        }
        assert_eq!(
            provider.gate_count(),
            0,
            "a finished fetch must leave no gate behind"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parallel_opens_for_cameras_the_list_lacks_pay_one_device_list() {
        let stub = Stub::with_delay(&["serial-one"], Duration::from_millis(300));
        let provider = Arc::new(provider(&stub));
        let held = Arc::new(session("probe", "access"));
        let mut pending = tokio::task::JoinSet::new();

        for serial in [
            "stranger-one",
            "stranger-two",
            "stranger-three",
            "stranger-four",
        ] {
            let provider = Arc::clone(&provider);
            let held = Arc::clone(&held);
            pending.spawn(async move { provider.owns(&held, serial).await });
        }
        while let Some(joined) = pending.join_next().await {
            assert!(
                !joined.expect("joined").expect("owns"),
                "a camera the account does not list is not owned"
            );
        }
        assert_eq!(
            stub.hits(),
            1,
            "one burst of unknown serials must ask the cloud once, not once per serial"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parallel_cold_opens_share_one_failed_attempt() {
        let stub = Stub::failing(&["serial-one"], 503, Duration::from_millis(300));
        let provider = Arc::new(provider(&stub));
        let held = Arc::new(session("probe", "access"));
        let started = Instant::now();
        let mut pending = tokio::task::JoinSet::new();

        for serial in ["serial-one", "serial-two", "serial-three", "serial-four"] {
            let provider = Arc::clone(&provider);
            let held = Arc::clone(&held);
            pending.spawn(async move { provider.owns(&held, serial).await });
        }
        while let Some(joined) = pending.join_next().await {
            let outcome = joined.expect("joined");
            assert!(
                outcome.is_err(),
                "a failed device list must surface as an error, not as an unowned camera"
            );
        }
        assert_eq!(
            stub.hits(),
            1,
            "four opens against one failing cloud must ask once"
        );
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "waiters must share the failure instead of queuing behind it, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_during_another_fetch_asks_the_cloud_again() {
        let stub = Stub::with_delay(&["serial-one"], Duration::from_millis(300));
        let provider = Arc::new(provider(&stub));
        let held = Arc::new(session("probe", "access"));

        let opener = {
            let provider = Arc::clone(&provider);
            let held = Arc::clone(&held);
            tokio::spawn(async move { provider.owns(&held, "serial-one").await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        stub.set_rows(&["serial-one", "serial-two"]);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let refreshed = provider.fetch_entries(&held).await.expect("refresh");
        assert!(
            refreshed.iter().any(|entry| entry.uuid == "serial-two"),
            "an explicit refresh must carry a camera bound while the other fetch ran"
        );
        assert!(opener.await.expect("joined").expect("owns"));
        assert_eq!(stub.hits(), 2, "the refresh is the second call");
    }

    #[tokio::test]
    async fn a_failed_attempt_does_not_wedge_the_session() {
        let stub = Stub::failing(&["serial-one"], 503, Duration::from_millis(20));
        let provider = provider(&stub);
        let session = session("probe", "access");

        assert!(provider.owns(&session, "serial-one").await.is_err());
        stub.set_status(200);
        assert!(
            provider.owns(&session, "serial-one").await.expect("retry"),
            "the next open must meet the cloud again, not a remembered failure"
        );
        assert_eq!(stub.hits(), 2, "the retry is the second call");
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
            .token_login(&mut connection, "stale-token", &control, SESSION_KEY)
            .await;
        assert!(matches!(ret, Ok(999)), "{ret:?}");
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

    #[tokio::test]
    async fn a_token_login_adopts_the_session_the_reply_body_names() {
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
                            "SessionID": "0x00000157"
                        })),
                    ),
                ];
            }
            Vec::new()
        })
        .await;
        let mut connection = connect(&address).await;
        let control = crate::media::dhav::MediaCipher::channel();
        let ret = provider()
            .token_login(&mut connection, "good-token", &control, SESSION_KEY)
            .await
            .expect("token login");
        assert_eq!(ret, 100);
        assert_eq!(
            connection.session(),
            0x157,
            "the claims session in the header must give way to the session the device names"
        );
        drop(connection);
        peer.await.expect("peer task");
    }

    const SESSION_KEY: &str = "abcdefghij012345";

    struct DeviceEnv;

    impl Drop for DeviceEnv {
        fn drop(&mut self) {
            std::env::remove_var("PROVIEW_DEVICE_SERIAL");
            std::env::remove_var("PROVIEW_DEVICE_USER");
            std::env::remove_var("PROVIEW_DEVICE_PASS");
        }
    }

    fn device_env(serial: &str, user: &str, pass: &str) -> DeviceEnv {
        std::env::set_var("PROVIEW_DEVICE_SERIAL", serial);
        std::env::set_var("PROVIEW_DEVICE_USER", user);
        std::env::set_var("PROVIEW_DEVICE_PASS", pass);
        DeviceEnv
    }

    fn control() -> crate::media::dhav::MediaCipher {
        crate::media::dhav::MediaCipher::channel()
    }

    const SERIAL: &str = "serial-tunnel-reopen";

    struct TokenDir(std::path::PathBuf);

    impl TokenDir {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("proview-tokens-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("token dir");
            Self(dir)
        }
    }

    impl Drop for TokenDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
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

    fn provider_with_store(dir: &TokenDir) -> CloudProvider {
        let mut provider = provider();
        provider.tokens = crate::cloud::DeviceTokenStore::load(Some(&dir.0));
        provider
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
                if let Some(plain) = crate::media::dhav::MediaCipher::channel().decrypt_body(&body)
                {
                    let plain = crate::media::dhav::trim_nul(&plain);
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(plain) {
                        if let Some(user) =
                            value.get("UserName").and_then(serde_json::Value::as_str)
                        {
                            note(&events, format!("conn{index}:token_user={user}"));
                        }
                    }
                }
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
    async fn device_login_keeps_the_tunnel_when_the_stored_token_is_accepted() {
        let peer = LoginPeer::start(100, Duration::from_millis(700)).await;
        let dir = TokenDir::new("accepted");
        let provider = provider_with_store(&dir);
        provider
            .tokens
            .put(SERIAL, "live-token")
            .expect("stored token");
        let first = connect(&peer.address).await;
        let (connection, note) = provider
            .device_login(first, &entry(SERIAL), &control(), SESSION_KEY)
            .await
            .expect("the token login finishes the session");
        assert_eq!(connection.session(), 0x3333_3333);
        assert!(note.contains("stored device token ret 100"), "{note}");
        let events = peer.events();
        assert_eq!(counted(&events, ":token_login"), 1, "{events:?}");
        assert_eq!(counted(&events, ":password_login"), 0, "{events:?}");
        drop(connection);
    }

    fn sealed_login_token(serial: &str, token: &str) -> String {
        use aes::cipher::{BlockEncrypt, KeyInit};
        use aes::{Aes128, Block};
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let mut key = [0u8; 16];
        key.copy_from_slice(&serial.as_bytes()[..16]);
        key.reverse();
        let cipher = Aes128::new_from_slice(&key).expect("aes-128 key");
        let mut buffer = token.as_bytes().to_vec();
        let pad = 16 - buffer.len() % 16;
        buffer.extend(std::iter::repeat_n(pad as u8, pad));
        for block in buffer.chunks_mut(16) {
            cipher.encrypt_block(Block::from_mut_slice(block));
        }
        STANDARD.encode(buffer)
    }

    #[tokio::test]
    async fn the_account_device_token_is_opened_before_the_token_login() {
        let peer = LoginPeer::start(100, Duration::from_millis(10)).await;
        let dir = TokenDir::new("account-token");
        let provider = provider_with_store(&dir);
        let serial = "0123456789abcdefghij";
        let token = "PROBElogin000001";
        let sealed = sealed_login_token(serial, token);
        let mut device = entry(serial);
        device.extra.insert(
            "deviceToken".to_string(),
            serde_json::json!({ "AdminToken": sealed }),
        );
        let first = connect(&peer.address).await;
        let (connection, note) = provider
            .device_login(first, &device, &control(), SESSION_KEY)
            .await
            .expect("the account token opens the session");
        assert!(note.contains("account device token ret 100"), "{note}");
        assert_eq!(connection.session(), 0x3333_3333);
        let events = peer.events();
        assert!(
            events.contains(&format!("conn1:token_user={token}")),
            "{events:?}"
        );
        assert_eq!(
            provider.tokens.get(serial).as_deref(),
            Some(sealed.as_str()),
            "the raw account token is kept for the next session"
        );
        drop(connection);
    }

    #[tokio::test]
    async fn a_stored_device_token_is_opened_before_the_token_login() {
        let peer = LoginPeer::start(100, Duration::from_millis(10)).await;
        let dir = TokenDir::new("opened-stored");
        let provider = provider_with_store(&dir);
        let serial = "0123456789abcdefghij";
        let token = "PROBElogin000001";
        let sealed = sealed_login_token(serial, token);
        provider.tokens.put(serial, &sealed).expect("stored token");
        let device = entry(serial);
        let first = connect(&peer.address).await;
        let (connection, note) = provider
            .device_login(first, &device, &control(), SESSION_KEY)
            .await
            .expect("the stored token opens the session");
        assert!(note.contains("stored device token ret 100"), "{note}");
        let events = peer.events();
        assert!(
            events.contains(&format!("conn1:token_user={token}")),
            "{events:?}"
        );
        drop(connection);
    }

    #[tokio::test]
    async fn a_stored_value_that_does_not_open_is_offered_untouched() {
        for (tag, stored) in [
            ("already-opened", "PROBElogin000001"),
            ("not-base64", "live-token"),
        ] {
            let peer = LoginPeer::start(100, Duration::from_millis(10)).await;
            let dir = TokenDir::new(tag);
            let provider = provider_with_store(&dir);
            provider.tokens.put(SERIAL, stored).expect("stored token");
            let first = connect(&peer.address).await;
            let (connection, note) = provider
                .device_login(first, &entry(SERIAL), &control(), SESSION_KEY)
                .await
                .expect("a value the decoder refuses is still offered as it stands");
            assert!(
                note.contains("stored device token ret 100"),
                "{tag}: {note}"
            );
            assert_eq!(connection.session(), 0x3333_3333, "{tag}");
            let events = peer.events();
            assert!(
                events.contains(&format!("conn1:token_user={stored}")),
                "{tag}: {events:?}"
            );
            assert_eq!(
                provider.tokens.get(SERIAL).as_deref(),
                Some(stored),
                "{tag}: the store must not be rewritten"
            );
            drop(connection);
        }
    }

    fn counted(events: &[String], needle: &str) -> usize {
        events.iter().filter(|event| event.contains(needle)).count()
    }

    #[tokio::test]
    async fn a_refusal_on_one_device_leaves_the_token_path_for_another() {
        const OTHER: &str = "serial-two";
        let refused = LoginPeer::start(137, Duration::from_millis(700)).await;
        let accepted = LoginPeer::start(100, Duration::from_millis(700)).await;
        let dir = TokenDir::new("mixed");
        let provider = provider_with_store(&dir);
        provider
            .tokens
            .put(SERIAL, "refused-token")
            .expect("stored token");
        provider
            .tokens
            .put(OTHER, "live-token")
            .expect("stored token");

        let first = connect(&refused.address).await;
        let error = provider
            .device_login(first, &entry(SERIAL), &control(), SESSION_KEY)
            .await
            .map(|_| ())
            .expect_err("a device that refuses the token cannot log in");
        assert!(error.to_string().contains("ret 137"), "{error}");

        let second = connect(&accepted.address).await;
        let (connection, note) = provider
            .device_login(second, &entry(OTHER), &control(), SESSION_KEY)
            .await
            .expect("the accepting device keeps the token path");
        assert!(note.contains("stored device token ret 100"), "{note}");
        assert_eq!(connection.session(), 0x3333_3333);
        let events = accepted.events();
        assert_eq!(counted(&events, ":token_login"), 1, "{events:?}");
        assert_eq!(counted(&events, ":password_login"), 0, "{events:?}");
        drop(connection);
    }
    #[tokio::test]
    async fn a_cloud_login_never_reads_the_device_credentials() {
        let _env = device_env(SERIAL, "admin", "hunter2");
        let peer = LoginPeer::start(100, Duration::from_millis(10)).await;
        let dir = TokenDir::new("no-credentials");
        let provider = provider_with_store(&dir);
        let first = connect(&peer.address).await;
        let error = provider
            .device_login(first, &entry(SERIAL), &control(), SESSION_KEY)
            .await
            .map(|_| ())
            .expect_err("an account without a device token cannot log in");
        let message = error.to_string();
        assert!(message.contains("stage=token"), "{message}");
        assert!(
            message.contains("account carries no device token"),
            "{message}"
        );
        assert!(!message.contains("PROVIEW_DEVICE"), "{message}");
        let events = peer.events();
        assert_eq!(counted(&events, ":password_login"), 0, "{events:?}");
    }

    #[tokio::test]
    async fn a_token_refusal_reports_the_stage_and_the_ret() {
        let peer = LoginPeer::start(205, Duration::from_millis(10)).await;
        let dir = TokenDir::new("ret-205");
        let provider = provider_with_store(&dir);
        provider
            .tokens
            .put(SERIAL, "token-the-device-refuses")
            .expect("stored token");
        let first = connect(&peer.address).await;
        let error = provider
            .device_login(first, &entry(SERIAL), &control(), SESSION_KEY)
            .await
            .map(|_| ())
            .expect_err("a refusal cannot log in");
        let message = error.to_string();
        assert!(message.contains("stage=token"), "{message}");
        assert!(message.contains("ret 205"), "{message}");
        assert!(message.contains("ip locked"), "{message}");
        assert!(
            provider.tokens.get(SERIAL).is_none(),
            "a refused token is dropped"
        );
        assert!(
            !super::open_is_worth_retrying(&error),
            "a device refusal is not a dead tunnel: {message}"
        );
        assert!(
            provider.token_refusal(SERIAL).is_none(),
            "an ip lock is the device's state, not a property of the token"
        );
    }

    #[tokio::test]
    async fn a_refused_token_is_not_offered_again_in_this_process() {
        let peer = LoginPeer::start(137, Duration::from_millis(10)).await;
        let dir = TokenDir::new("refused-once");
        let provider = provider_with_store(&dir);
        provider
            .tokens
            .put(SERIAL, "refused-token")
            .expect("stored token");
        let mut device = entry(SERIAL);
        device.extra.insert(
            "deviceToken".to_string(),
            serde_json::json!({ "AdminToken": "refused-token" }),
        );
        let first = connect(&peer.address).await;
        let opening = provider
            .device_login(first, &device, &control(), SESSION_KEY)
            .await
            .map(|_| ())
            .expect_err("the device refuses the token");
        assert!(opening.to_string().contains("ret 137"), "{opening}");
        let second = connect(&peer.address).await;
        let again = provider
            .device_login(second, &device, &control(), SESSION_KEY)
            .await
            .map(|_| ())
            .expect_err("the remembered refusal fails the next open");
        let message = again.to_string();
        assert!(message.contains("stage=token"), "{message}");
        assert!(message.contains("ret 137"), "{message}");
        assert!(message.contains("earlier in this process"), "{message}");
        let events = peer.events();
        assert_eq!(
            counted(&events, ":token_login"),
            1,
            "a device that refuses tokens must not be offered one twice: {events:?}"
        );
    }

    #[tokio::test]
    async fn a_new_account_token_is_offered_after_the_old_one_was_refused() {
        let refused = LoginPeer::start(137, Duration::from_millis(10)).await;
        let accepted = LoginPeer::start(100, Duration::from_millis(10)).await;
        let dir = TokenDir::new("fresh-account-token");
        let provider = provider_with_store(&dir);
        let mut old = entry(SERIAL);
        old.extra.insert(
            "deviceToken".to_string(),
            serde_json::json!({ "AdminToken": "old-token" }),
        );
        let first = connect(&refused.address).await;
        let opening = provider
            .device_login(first, &old, &control(), SESSION_KEY)
            .await
            .map(|_| ())
            .expect_err("the device refuses the first token");
        assert!(opening.to_string().contains("ret 137"), "{opening}");
        let mut fresh = entry(SERIAL);
        fresh.extra.insert(
            "deviceToken".to_string(),
            serde_json::json!({ "AdminToken": "new-token" }),
        );
        let second = connect(&accepted.address).await;
        let (connection, note) = provider
            .device_login(second, &fresh, &control(), SESSION_KEY)
            .await
            .expect("a token the account issues after the refusal must still be tried");
        assert!(note.contains("account device token ret 100"), "{note}");
        assert_eq!(connection.session(), 0x3333_3333);
        let events = accepted.events();
        assert_eq!(counted(&events, ":token_login"), 1, "{events:?}");
        drop(connection);
    }

    #[test]
    fn a_device_refusal_is_not_retried_but_a_dead_tunnel_is() {
        assert!(!super::open_is_worth_retrying(
            &ProviderError::LoginRefused("ret 205 ip locked".to_string())
        ));
        assert!(!super::open_is_worth_retrying(
            &ProviderError::Misconfigured("no identity".to_string())
        ));
        assert!(!super::open_is_worth_retrying(
            &ProviderError::UnknownDevice("x".to_string())
        ));
        assert!(super::open_is_worth_retrying(&ProviderError::Upstream(
            "control session: Frame(NotEnoughData)".to_string()
        )));
        assert!(super::open_is_worth_retrying(&ProviderError::Unreachable(
            "relay".to_string()
        )));
        assert!(super::open_is_worth_retrying(&ProviderError::Io(
            std::io::Error::other("reset")
        )));
    }
}
