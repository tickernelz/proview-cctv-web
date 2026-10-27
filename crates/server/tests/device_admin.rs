use std::path::PathBuf;

use proview_core::provider::Mode;
use proview_server::config::ModeStore;
use rocket::http::{ContentType, Status};
use rocket::local::blocking::Client;
use serde_json::json;

fn data_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvw-device-admin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("PROVIEW_DATA_DIR", &dir);
    dir
}

fn config() -> proview_core::config::Config {
    proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    }
}

fn client() -> Client {
    Client::tracked(proview_server::build_with_modes(
        rocket::Config::figment()
            .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
            .merge(("port", 0)),
        config(),
        ModeStore::new(Mode::Cloud, None),
    ))
    .expect("valid rocket instance")
}

fn sign_in(client: &Client) {
    let status = client
        .post("/api/login")
        .header(ContentType::JSON)
        .body(
            json!({ "email": "fixture@example.invalid", "password": "fixture-password" })
                .to_string(),
        )
        .dispatch()
        .status();
    assert_eq!(status, Status::Ok);
}

fn body(response: rocket::local::blocking::LocalResponse<'_>) -> serde_json::Value {
    serde_json::from_str(&response.into_string().unwrap()).unwrap()
}

#[test]
fn every_admin_route_refuses_a_call_without_a_session() {
    let client = client();
    assert_eq!(
        client
            .get("/api/cloud/device-attributes")
            .dispatch()
            .status(),
        Status::Unauthorized
    );
    for path in [
        "/api/cloud/device-groups",
        "/api/cloud/device/share/pending?uuids=SN-1",
        "/api/cloud/device/share/info?key=abc",
    ] {
        assert_eq!(
            client.get(path).dispatch().status(),
            Status::Unauthorized,
            "{path}"
        );
    }
    let calls = [
        (
            "/api/cloud/device/rename",
            json!({ "uuid": "SN-1", "nickname": "pintu" }),
        ),
        (
            "/api/cloud/device/group",
            json!({ "uuid": "SN-1", "group": "pintu" }),
        ),
        (
            "/api/cloud/device/favourite",
            json!({ "uuid": "SN-1", "favourite": true }),
        ),
        (
            "/api/cloud/device/share",
            json!({ "uuids": ["SN-1"], "acceptId": "other" }),
        ),
        ("/api/cloud/device/share/accept", json!({ "key": "abc" })),
        (
            "/api/cloud/device/share/cancel",
            json!({ "key": "abc", "uuids": ["SN-1"] }),
        ),
        (
            "/api/cloud/device/password",
            json!({ "uuid": "SN-1", "password": "lama", "newPassword": "baru" }),
        ),
        (
            "/api/device/password",
            json!({ "newPassword": "baru-sekali" }),
        ),
    ];
    for (path, payload) in &calls {
        let status = client
            .post(*path)
            .header(ContentType::JSON)
            .body(payload.to_string())
            .dispatch()
            .status();
        assert_eq!(status, Status::Unauthorized, "{path}");
    }
    let guarded = 1 + 3 + calls.len();
    assert_eq!(
        guarded,
        proview_server::routes::device_admin::routes().len(),
        "every route in device_admin::routes() must appear in this test"
    );
}

#[test]
fn a_local_session_is_told_the_cloud_is_what_carries_admin() {
    let client = client();
    sign_in(&client);
    let response = client
        .post("/api/cloud/device/rename")
        .header(ContentType::JSON)
        .body(json!({ "uuid": "SN-1", "nickname": "pintu" }).to_string())
        .dispatch();
    assert_eq!(response.status(), Status::Conflict);
    let reply = body(response);
    assert!(
        reply["message"]
            .as_str()
            .unwrap_or_default()
            .contains("local device"),
        "{reply}"
    );
}

#[test]
fn a_group_and_a_favourite_survive_a_restart() {
    let dir = data_dir();
    let first = client();
    sign_in(&first);
    for (path, payload) in [
        (
            "/api/cloud/device/group",
            json!({ "uuid": "SN-ROUNDTRIP", "group": "pintu" }),
        ),
        (
            "/api/cloud/device/favourite",
            json!({ "uuid": "SN-ROUNDTRIP", "favourite": true }),
        ),
    ] {
        let status = first
            .post(path)
            .header(ContentType::JSON)
            .body(payload.to_string())
            .dispatch()
            .status();
        assert_eq!(status, Status::Ok, "{path}");
    }

    let saved = std::fs::read_to_string(dir.join("device-groups.json")).expect("the store file");
    assert!(saved.contains("pintu"), "{saved}");
    assert!(saved.contains("SN-ROUNDTRIP"), "{saved}");

    let second = client();
    sign_in(&second);
    let reply = body(second.get("/api/cloud/device-groups").dispatch());
    assert_eq!(reply["groups"], json!(["pintu"]));

    let cleared = second
        .post("/api/cloud/device/group")
        .header(ContentType::JSON)
        .body(json!({ "uuid": "SN-ROUNDTRIP", "group": "" }).to_string())
        .dispatch()
        .status();
    assert_eq!(cleared, Status::Ok);
    let reply = body(second.get("/api/cloud/device-groups").dispatch());
    assert_eq!(reply["groups"], json!([]));

    let mut handles = Vec::new();
    for worker in 0..8 {
        handles.push(std::thread::spawn(move || {
            let client = client();
            sign_in(&client);
            for step in 0..5 {
                let uuid = format!("SN-CONCURRENT-{worker}-{step}");
                let status = client
                    .post("/api/cloud/device/group")
                    .header(ContentType::JSON)
                    .body(json!({ "uuid": uuid, "group": uuid }).to_string())
                    .dispatch()
                    .status();
                assert_eq!(status, Status::Ok);
            }
        }));
    }
    for handle in handles {
        handle.join().expect("a writer thread");
    }
    let saved = std::fs::read_to_string(dir.join("device-groups.json")).expect("the store file");
    for worker in 0..8 {
        for step in 0..5 {
            let uuid = format!("SN-CONCURRENT-{worker}-{step}");
            assert!(saved.contains(&uuid), "{uuid} was lost: {saved}");
        }
    }

    std::fs::write(dir.join("device-groups.json"), "{\"devices\": {\"SN-\"").unwrap();
    let broken = client();
    sign_in(&broken);
    let response = broken.get("/api/cloud/device-groups").dispatch();
    assert_eq!(response.status(), Status::InternalServerError);
    let kept = std::fs::read_to_string(dir.join("device-groups.json")).unwrap();
    assert_eq!(
        kept, "{\"devices\": {\"SN-\"",
        "a broken store is never overwritten"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_dvrip_password_route_names_the_missing_camera() {
    let client = client();
    sign_in(&client);
    let response = client
        .post("/api/device/password")
        .header(ContentType::JSON)
        .body(json!({ "newPassword": "baru-sekali" }).to_string())
        .dispatch();
    assert_eq!(response.status(), Status::BadRequest);
    let reply = body(response);
    assert!(
        reply["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no camera address is set"),
        "{reply}"
    );
}
