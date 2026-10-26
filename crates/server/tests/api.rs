use proview_core::provider::Mode;
use proview_server::config::ModeStore;
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
        device_onvif: None,
        device_rtsp: local.then(|| "192.0.2.10:554".to_string()),
        session_ttl_secs: 3600,
    }
}

fn client() -> Client {
    Client::tracked(proview_server::build_with_modes(
        figment(),
        config(false),
        ModeStore::new(Mode::Cloud, None),
    ))
    .expect("valid rocket instance")
}

fn local_client() -> Client {
    Client::tracked(proview_server::build_with_modes(
        figment(),
        config(true),
        ModeStore::new(Mode::Cloud, None),
    ))
    .expect("valid rocket instance")
}

fn figment() -> rocket::figment::Figment {
    rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0))
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

fn body(response: rocket::local::blocking::LocalResponse<'_>) -> serde_json::Value {
    serde_json::from_str(&response.into_string().unwrap()).unwrap()
}

fn choose_mode<'a>(client: &'a Client, mode: &str) -> rocket::local::blocking::LocalResponse<'a> {
    client
        .post("/api/config/mode")
        .header(ContentType::JSON)
        .body(json!({ "mode": mode }).to_string())
        .dispatch()
}

#[test]
fn the_mode_defaults_to_cloud_before_any_session_exists() {
    let client = client();
    let response = client.get("/api/config/mode").dispatch();
    assert_eq!(response.status(), Status::Ok);
    let report = body(response);
    assert_eq!(report["mode"], "cloud");
    assert_eq!(report["source"], "fixture");
    assert!(report["notice"].is_null());
}

#[test]
fn local_mode_lists_the_lan_camera_and_names_the_stream_source() {
    let client = local_client();
    assert_eq!(sign_in(&client), Status::Ok);

    let chosen = choose_mode(&client, "local");
    assert_eq!(chosen.status(), Status::Ok);
    let report = body(chosen);
    assert_eq!(report["mode"], "local");
    assert_eq!(report["source"], "local");
    assert!(report["notice"].is_null());

    let devices = client.get("/api/devices").dispatch();
    assert_eq!(devices.status(), Status::Ok);
    let list = body(devices);
    assert_eq!(list[0]["id"], "local");
    assert_eq!(list[0]["lan_host"], "192.0.2.10:554");

    let ticket = client
        .get("/api/stream?device=local&channel=1&quality=main")
        .dispatch();
    assert_eq!(ticket.status(), Status::Ok);
    let body = body(ticket);
    assert_eq!(body["kind"], "ws");
    assert_eq!(body["url"], "/ws/live?device=local&channel=1&quality=main");
    assert_eq!(body["source"], "local");
}

#[test]
fn choosing_local_without_a_host_reports_the_cloud_fallback() {
    let client = client();
    assert_eq!(sign_in(&client), Status::Ok);

    let chosen = choose_mode(&client, "local");
    assert_eq!(chosen.status(), Status::Ok);
    let report = body(chosen);
    assert_eq!(report["mode"], "local");
    assert_eq!(report["source"], "fixture");
    assert_eq!(report["notice"]["from"], "local");
    assert_eq!(report["notice"]["to"], "fixture");
    let message = report["notice"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("PROVIEW_DEVICE_RTSP"),
        "the notice names the missing setting: {message}"
    );

    let devices = client.get("/api/devices").dispatch();
    assert_eq!(devices.status(), Status::Ok);
    assert_eq!(body(devices)[0]["id"], "fixture-dev-0001");

    let again = client.get("/api/config/mode").dispatch();
    assert_eq!(body(again)["source"], "fixture");
}

#[test]
fn a_mode_choice_needs_a_session_and_ends_with_it() {
    let client = client();
    assert_eq!(choose_mode(&client, "local").status(), Status::Unauthorized);

    assert_eq!(sign_in(&client), Status::Ok);
    assert_eq!(choose_mode(&client, "local").status(), Status::Ok);
    assert_eq!(client.post("/api/logout").dispatch().status(), Status::Ok);

    let report = body(client.get("/api/config/mode").dispatch());
    assert_eq!(report["mode"], "cloud");
}

#[test]
fn the_login_message_names_the_provider_that_answered() {
    let client = client();
    let response = client
        .post("/api/login")
        .header(ContentType::JSON)
        .body(
            json!({ "email": "fixture@example.invalid", "password": "fixture-password" })
                .to_string(),
        )
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    let message = body(response)["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(message.contains("signed in via fixture"), "{message}");
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

#[test]
fn control_routes_require_a_session() {
    let client = client();
    assert_eq!(
        client.get("/api/device/controls").dispatch().status(),
        Status::Unauthorized
    );
    assert_eq!(
        client
            .post("/api/device/controls/ptz?pan=0.5")
            .dispatch()
            .status(),
        Status::Unauthorized
    );
}

#[test]
fn capability_report_lists_both_backends_as_unconfigured() {
    let client = client();
    assert_eq!(sign_in(&client), Status::Ok);

    let response = client.get("/api/device/controls").dispatch();
    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&response.into_string().unwrap()).unwrap();
    assert_eq!(body["onvif"], false);
    assert_eq!(body["dvrip"], false);
    let rows = body["capabilities"].as_array().unwrap();
    assert!(rows.iter().any(|row| row["feature"] == "onvif transport"
        && row["state"] == "blocked"
        && row["backend"] == "onvif"
        && row["lens"] == "device"));
    assert!(rows
        .iter()
        .any(|row| row["feature"] == "dvrip settings" && row["state"] == "blocked"));
}

#[test]
fn ptz_move_needs_a_velocity_and_a_known_lens() {
    let client = client();
    assert_eq!(sign_in(&client), Status::Ok);

    let no_velocity = client
        .post("/api/device/controls/ptz?lens=bottom")
        .dispatch();
    assert_eq!(no_velocity.status(), Status::BadRequest);

    let unknown_lens = client
        .post("/api/device/controls/ptz?lens=middle&pan=0.4")
        .dispatch();
    assert_eq!(unknown_lens.status(), Status::BadRequest);

    let no_backend = client
        .post("/api/device/controls/ptz?lens=bottom&pan=0.4")
        .dispatch();
    assert_eq!(no_backend.status(), Status::BadGateway);
}

#[test]
fn live_stats_needs_a_session_and_starts_empty() {
    let client = local_client();
    assert_eq!(
        client.get("/api/live/stats").dispatch().status(),
        Status::Unauthorized
    );
    assert_eq!(sign_in(&client), Status::Ok);
    let response = client.get("/api/live/stats").dispatch();
    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&response.into_string().unwrap()).unwrap();
    assert_eq!(body["active_streams"], 0);
    assert_eq!(body["streams"].as_array().map(Vec::len), Some(0));
}
