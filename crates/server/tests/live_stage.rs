use std::sync::Arc;
use std::time::Duration;

use proview_core::provider::{Mode, VendorSession};
use proview_core::session::Store;
use proview_server::config::ModeStore;
use rocket::http::{Cookie, CookieJar, Status};
use rocket::local::blocking::Client;
use rocket::post;
use rocket::routes;
use rocket::State;
use serde_json::Value;

fn config() -> proview_core::config::Config {
    proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Proview,
        cloud_base: "http://127.0.0.1:1".to_string(),
        share_base: proview_core::cloud::attributes::SHARE_BASE.to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    }
}

#[post("/_test/session")]
fn seed_session(store: &State<Store>, cookies: &CookieJar<'_>) -> String {
    let id = proview_core::session::new_id();
    store.put(
        id.clone(),
        VendorSession {
            account: "walk@example.invalid".to_string(),
            token: Some("cloud-token".to_string()),
        },
        Duration::from_secs(600),
    );
    cookies.add_private(Cookie::build(("pvw_session", id.clone())).build());
    id
}

fn client() -> Client {
    let figment = rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0));
    let config = config();
    let cloud = Arc::new(proview_core::provider::cloud::CloudProvider::new(&config));
    let rig = rocket::custom(figment)
        .manage(config)
        .manage(cloud)
        .manage(Store::new())
        .manage(ModeStore::new(Mode::Cloud, None))
        .manage(proview_server::live_state::LiveState::new())
        .mount(
            "/api",
            routes![
                proview_server::routes::live::stream,
                proview_server::routes::live::stats,
                seed_session
            ],
        );
    Client::tracked(rig).expect("valid rocket instance")
}

#[test]
fn a_stream_that_dies_at_the_device_list_names_that_stage() {
    let client = client();
    assert_eq!(
        client.post("/api/_test/session").dispatch().status(),
        Status::Ok
    );

    let response = client
        .get("/api/stream?device=fixture-dev-0001&channel=1&quality=main")
        .dispatch();
    assert_eq!(response.status(), Status::BadGateway);
    let body: Value = serde_json::from_str(&response.into_string().unwrap()).unwrap();

    assert_eq!(body["ok"], false);
    assert_eq!(body["stage"], "device_list");
    assert!(body["elapsed_ms"].is_u64());
    assert!(
        !body["detail"].as_str().unwrap_or_default().is_empty(),
        "the failing stage must carry the transport's own words: {body}"
    );
    assert!(body["message"]
        .as_str()
        .unwrap_or_default()
        .contains("127.0.0.1:1"));

    let stages = body["stages"].as_array().expect("stage list");
    assert_eq!(stages.len(), 2, "{body}");
    assert_eq!(stages[0]["stage"], "cloud_login");
    assert_eq!(stages[0]["ok"], true);
    assert!(stages[0]["elapsed_ms"].is_u64());
    assert_eq!(stages[1]["stage"], "device_list");
    assert_eq!(stages[1]["ok"], false);

    let stats: Value = serde_json::from_str(
        &client
            .get("/api/live/stats")
            .dispatch()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stats["frames"], 0);
    assert_eq!(stats["bytes"], 0);
    assert_eq!(stats["attempts"][0]["device"], "fixture-dev-0001");
    assert_eq!(stats["attempts"][0]["stage"], "device_list");
}

#[test]
fn the_stage_report_needs_a_session() {
    let client = client();
    assert_eq!(
        client
            .get("/api/stream?device=fixture-dev-0001&channel=1&quality=main")
            .dispatch()
            .status(),
        Status::Unauthorized
    );
    assert_eq!(
        client.get("/api/live/stats").dispatch().status(),
        Status::Unauthorized
    );
}
