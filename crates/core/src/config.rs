use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: ProviderKind,
    pub cloud_base: String,
    pub device_user: String,
    pub device_pass: String,
    pub fixture_dir: String,
    pub device_host: Option<String>,
    pub device_rtsp: Option<String>,
    pub device_onvif: Option<String>,
    pub session_ttl_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Fixture,
    Proview,
}

fn default_fixture_dir() -> String {
    if std::path::Path::new("fixtures").is_dir() {
        return "fixtures".to_string();
    }
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string()
}

impl Config {
    pub fn from_env() -> Self {
        let provider = match env::var("PROVIEW_PROVIDER").as_deref() {
            Ok("proview") => ProviderKind::Proview,
            _ => ProviderKind::Fixture,
        };
        Self {
            provider,
            cloud_base: env::var("PROVIEW_CLOUD_BASE")
                .unwrap_or_else(|_| "https://pub-token.xmeye.net".to_string()),
            device_user: env::var("PROVIEW_DEVICE_USER").unwrap_or_else(|_| "admin".to_string()),
            device_pass: env::var("PROVIEW_DEVICE_PASS").unwrap_or_default(),
            fixture_dir: env::var("PROVIEW_FIXTURE_DIR").unwrap_or_else(|_| default_fixture_dir()),
            device_host: env::var("PROVIEW_DEVICE_HOST")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            device_onvif: env::var("PROVIEW_DEVICE_ONVIF")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            device_rtsp: env::var("PROVIEW_DEVICE_RTSP")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            session_ttl_secs: env::var("PROVIEW_SESSION_TTL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
        }
    }
}
