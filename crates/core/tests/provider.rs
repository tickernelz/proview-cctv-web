use proview_core::config::{Config, ProviderKind};
use proview_core::provider::{
    self, local::LocalProvider, Mode, Provider, ProviderError, ProviderSource, Router,
};
use proview_core::source::RtspTarget;

fn config(kind: ProviderKind, device_rtsp: Option<&str>) -> Config {
    Config {
        provider: kind,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_onvif: None,
        device_host: None,
        device_rtsp: device_rtsp.map(str::to_string),
        session_ttl_secs: 3600,
    }
}

fn fixture_config() -> Config {
    config(ProviderKind::Fixture, None)
}

#[tokio::test]
async fn a_cloud_login_without_platform_keys_is_a_configuration_error() {
    let cfg = config(ProviderKind::Proview, None);
    let provider = provider::cloud::CloudProvider::with_identity(
        &cfg,
        proview_core::cloud::Identity::default(),
    );
    let error = provider
        .login("someone", "secret")
        .await
        .expect_err("no platform keys");
    assert!(
        matches!(error, ProviderError::Misconfigured(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("PROVIEW_APP_KEY"), "{error}");
}

#[tokio::test]
async fn a_missing_fixture_directory_says_where_it_looked() {
    let mut cfg = fixture_config();
    cfg.fixture_dir = "/nonexistent-fixture-directory".to_string();
    let provider = provider::fixture::Fixture::new(&cfg);
    let error = provider
        .login("someone", "secret")
        .await
        .expect_err("no fixture account");
    let message = error.to_string();
    assert!(
        message.contains("/nonexistent-fixture-directory/account.json"),
        "{message}"
    );
    assert!(matches!(error, ProviderError::Misconfigured(_)));
}

fn cloud_config(device_rtsp: Option<&str>) -> Config {
    config(ProviderKind::Proview, device_rtsp)
}

#[test]
fn the_mode_defaults_to_cloud() {
    assert_eq!(Mode::default(), Mode::Cloud);
    assert_eq!(Mode::parse("cloud"), Some(Mode::Cloud));
    assert_eq!(Mode::parse(" LOCAL "), Some(Mode::Local));
    assert_eq!(Mode::parse("auto"), None);
}

#[test]
fn cloud_mode_plans_the_configured_provider_without_a_notice() {
    let cloud = provider::plan(&cloud_config(None), Mode::default());
    assert_eq!(cloud.mode, Mode::Cloud);
    assert_eq!(cloud.source, ProviderSource::Cloud);
    assert!(cloud.notice.is_none());

    let fixture = provider::plan(&fixture_config(), Mode::Cloud);
    assert_eq!(fixture.source, ProviderSource::Fixture);
    assert!(fixture.notice.is_none());
}

#[test]
fn local_mode_without_a_host_plans_the_cloud_fallback() {
    let plan = provider::plan(&fixture_config(), Mode::Local);
    assert_eq!(plan.mode, Mode::Local);
    assert_eq!(plan.source, ProviderSource::Fixture);
    let notice = plan.notice.expect("a fallback notice is reported");
    assert_eq!(notice.from, Mode::Local);
    assert_eq!(notice.to, ProviderSource::Fixture);
    assert!(
        !notice.message.is_empty() && !notice.message.contains("PROVIEW_"),
        "the fallback notice explains the fix without a raw key: {}",
        notice.message
    );
}

#[test]
fn local_mode_with_a_host_plans_the_local_provider() {
    let plan = provider::plan(&cloud_config(Some("192.168.0.100:554")), Mode::Local);
    assert_eq!(plan.source, ProviderSource::Local);
    assert!(plan.notice.is_none());
}

#[test]
fn a_router_in_local_mode_builds_the_local_provider_first() {
    let router = Router::new(&cloud_config(Some("192.0.2.10:554")), Mode::Local);
    let resolution = router.resolution();
    assert_eq!(resolution.mode, Mode::Local);
    assert_eq!(resolution.source, ProviderSource::Local);
    assert!(resolution.notice.is_none());
    assert_eq!(router.source(), ProviderSource::Local);
}

#[test]
fn a_router_without_a_local_host_serves_cloud_and_explains_why() {
    let router = Router::new(&fixture_config(), Mode::Local);
    let resolution = router.resolution();
    assert_eq!(resolution.mode, Mode::Local);
    assert_eq!(resolution.source, ProviderSource::Fixture);
    assert!(resolution.notice.is_some());
}

#[tokio::test]
async fn an_unreachable_local_host_falls_back_to_the_cloud_provider() {
    let cfg = fixture_config();
    let unreachable = LocalProvider::with_target(RtspTarget::from_host_port("127.0.0.1", 1), 3600);
    let router = Router::with(
        Box::new(unreachable),
        Some(provider::build(&cfg)),
        Mode::Local,
    );

    let session = router
        .login("fixture@example.invalid", "fixture-password")
        .await
        .expect("the cloud provider accepts the fixture account");

    assert_eq!(session.account, "fixture@example.invalid");
    let resolution = router.resolution();
    assert_eq!(resolution.source, ProviderSource::Fixture);
    let notice = resolution.notice.expect("the fallback is reported");
    assert_eq!(notice.from, Mode::Local);
    assert_eq!(notice.to, ProviderSource::Fixture);
    assert!(notice.message.contains("127.0.0.1:1"), "{}", notice.message);

    let devices = router
        .devices(&session)
        .await
        .expect("devices over the fallback");
    assert_eq!(devices[0].id, "fixture-dev-0001");
    assert_eq!(router.source(), ProviderSource::Fixture);
}

#[tokio::test]
async fn bad_credentials_never_fall_back_to_another_provider() {
    let cfg = fixture_config();
    let router = Router::with(
        provider::build(&cfg),
        Some(provider::build(&cfg)),
        Mode::Cloud,
    );

    let error = router
        .login("fixture@example.invalid", "wrong-password")
        .await
        .expect_err("the fixture rejects the password");
    assert!(matches!(error, ProviderError::BadCredentials));
    assert!(router.resolution().notice.is_none());
}

#[tokio::test]
async fn the_stream_ticket_names_its_source() {
    let dir = std::env::temp_dir().join(format!("proview-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("devices.json"),
        r#"[{"id":"fixture-dev-0001","name":"Fixture camera","model":"XMEye-Fixture","online":true,"channels":1,"lan_host":null}]"#,
    )
    .unwrap();
    std::fs::write(dir.join("fixture-dev-0001_ch1_main.h264"), b"annex-b").unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/account.json"),
        dir.join("account.json"),
    )
    .unwrap();

    let mut cfg = fixture_config();
    cfg.fixture_dir = dir.to_string_lossy().to_string();
    let fixture = provider::build(&cfg);
    let session = fixture
        .login("fixture@example.invalid", "fixture-password")
        .await
        .unwrap();
    let ticket = fixture
        .stream(&session, "fixture-dev-0001", 1, true)
        .await
        .unwrap();
    assert_eq!(ticket.source, ProviderSource::Fixture);
    std::fs::remove_dir_all(&dir).ok();

    let local = LocalProvider::with_target(RtspTarget::from_host_port("192.0.2.10", 554), 600);
    let ticket = local.stream(&session, "local", 1, true).await.unwrap();
    assert_eq!(ticket.source, ProviderSource::Local);
    assert_eq!(ticket.expires_in, 600);
    assert!(ticket.url.contains("quality=main"), "{}", ticket.url);
    assert!(local.stream(&session, "local", 2, true).await.is_err());
    assert!(local
        .stream(&session, "local&channel=9", 1, true)
        .await
        .is_err());
    assert!(local.stream(&session, "../etc", 1, true).await.is_err());
}
