use rocket::http::{ContentType, Status};
use rocket::local::blocking::Client;
use serde_json::json;

fn config(local: bool) -> proview_core::config::Config {
    proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_rtsp: local.then(|| "192.0.2.10:554".to_string()),
        session_ttl_secs: 3600,
    }
}

fn client() -> Client {
    let figment = rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0));
    Client::tracked(proview_server::build_with(figment, config(false)))
        .expect("valid rocket instance")
}

fn local_client() -> Client {
    let figment = rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0));
    Client::tracked(proview_server::build_with(figment, config(true)))
        .expect("valid rocket instance")
}

fn sign_in(client: &Client) -> Status {
    client
        .post("/api/login")
        .header(ContentType::JSON)
        .body(
            json!({ "email": "fixture@example.invalid", "password": "fixture-password" })
                .to_string(),
        )
        .dispatch()
        .status()
}

#[test]
fn local_mode_lists_the_lan_camera_and_returns_a_ws_ticket() {
    let client = local_client();
    assert_eq!(sign_in(&client), Status::Ok);

    let devices = client.get("/api/devices").dispatch();
    assert_eq!(devices.status(), Status::Ok);
    let list: serde_json::Value = serde_json::from_str(&devices.into_string().unwrap()).unwrap();
    assert_eq!(list[0]["id"], "local");
    assert_eq!(list[0]["lan_host"], "192.0.2.10:554");

    let ticket = client
        .get("/api/stream?device=local&channel=1&quality=main")
        .dispatch();
    assert_eq!(ticket.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&ticket.into_string().unwrap()).unwrap();
    assert_eq!(body["kind"], "ws");
    assert_eq!(body["url"], "/ws/live?device=local&channel=1&quality=main");
}

#[test]
fn devices_require_a_session() {
    let client = client();
    assert_eq!(
        client.get("/api/devices").dispatch().status(),
        Status::Unauthorized
    );
}

#[test]
fn wrong_password_is_rejected() {
    let client = client();
    let response = client
        .post("/api/login")
        .header(ContentType::JSON)
        .body(json!({ "email": "fixture@example.invalid", "password": "wrong" }).to_string())
        .dispatch();
    assert_eq!(response.status(), Status::Unauthorized);
}

#[test]
fn fixture_session_lists_devices_and_reports_the_account() {
    let client = client();
    assert_eq!(sign_in(&client), Status::Ok);

    let me = client.get("/api/me").dispatch();
    assert_eq!(me.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&me.into_string().unwrap()).unwrap();
    assert_eq!(body["account"], "fixture@example.invalid");

    let devices = client.get("/api/devices").dispatch();
    assert_eq!(devices.status(), Status::Ok);
    let list: serde_json::Value = serde_json::from_str(&devices.into_string().unwrap()).unwrap();
    assert_eq!(list[0]["id"], "fixture-dev-0001");
}

#[test]
fn stream_outside_the_channel_range_is_not_found() {
    let client = client();
    assert_eq!(sign_in(&client), Status::Ok);
    let response = client
        .get("/api/stream?device=fixture-dev-0001&channel=9&quality=main")
        .dispatch();
    assert_eq!(response.status(), Status::NotFound);
}

#[test]
fn device_probe_reports_a_configuration_error_before_any_network_call() {
    let client = client();
    let response = client.post("/api/device/probe").dispatch();
    let status = response.status();
    assert!(
        status == Status::BadRequest || status == Status::BadGateway,
        "unexpected status {status}"
    );
    if std::env::var("PROVIEW_DEVICE_HOST").is_err() {
        assert_eq!(status, Status::BadRequest);
    }
}

#[test]
fn logout_invalidates_the_session() {
    let client = client();
    assert_eq!(sign_in(&client), Status::Ok);
    assert_eq!(client.post("/api/logout").dispatch().status(), Status::Ok);
    assert_eq!(
        client.get("/api/devices").dispatch().status(),
        Status::Unauthorized
    );
}
