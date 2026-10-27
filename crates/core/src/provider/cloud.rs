use super::rps as rps_client;
use super::{
    p2p, Alarm, CloudPath, Device, DeviceSettings, Provider, ProviderError, ProviderSource,
    Recording, StreamKind, StreamTicket, VendorSession,
};
use crate::cloud::envelope::{EnvelopeClient, Identity, MISSING_CLOUD_IDENTITY};
use crate::cloud::status::{self, StatusClient, STATUS_HOST};
use crate::cloud::CloudError;
use crate::config::Config;

pub struct CloudProvider {
    client: EnvelopeClient,
    status: StatusClient,
    device_user: String,
    device_pass: String,
}

impl CloudProvider {
    pub fn new(cfg: &Config) -> Self {
        Self::with_identity(cfg, Identity::resolve())
    }

    pub fn with_identity(cfg: &Config, identity: Identity) -> Self {
        Self {
            client: EnvelopeClient::with_base(identity, cfg.cloud_base.clone()),
            status: StatusClient::new(STATUS_HOST),
            device_user: cfg.device_user.clone(),
            device_pass: cfg.device_pass.clone(),
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
        _device: &str,
        _channel: u32,
        _high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        let _ = (&self.device_user, &self.device_pass, StreamKind::Rtsp);
        Err(ProviderError::NotImplemented(
            "live stream needs the device session: LAN RTSP or CSS HLS, neither wired yet",
        ))
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
