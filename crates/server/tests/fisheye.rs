use proview_core::provider::Mode;
use proview_server::config::ModeStore;
use rocket::http::{ContentType, Status};
use rocket::local::blocking::Client;
use serde_json::json;

fn client() -> Client {
    let config = proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_onvif: None,
        device_rtsp: Some("192.0.2.10:554".to_string()),
        session_ttl_secs: 3600,
    };
    let figment = rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0));
    Client::tracked(proview_server::build_with_modes(
        figment,
        config,
        ModeStore::new(Mode::Cloud, None),
    ))
    .expect("valid rocket instance")
}

fn sign_in(client: &Client) {
    let response = client
        .post("/api/login")
        .header(ContentType::JSON)
        .body(
            json!({ "email": "fixture@example.invalid", "password": "fixture-password" })
                .to_string(),
        )
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
}

fn spoke_color(index: usize) -> [u8; 3] {
    let hue = index as f32 / 12.0;
    let sector = hue * 6.0;
    let base = sector.floor();
    let fraction = sector - base;
    let ascending = (fraction * 255.0).round() as u8;
    let descending = 255 - ascending;
    match base as u32 % 6 {
        0 => [255, ascending, 0],
        1 => [descending, 255, 0],
        2 => [0, 255, ascending],
        3 => [0, descending, 255],
        4 => [ascending, 0, 255],
        _ => [255, 0, descending],
    }
}

struct Bitmap {
    width: u32,
    height: u32,
    stride: usize,
    pixels: Vec<u8>,
}

impl Bitmap {
    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 54 || &bytes[0..2] != b"BM" {
            return None;
        }
        let offset = u32::from_le_bytes(bytes[10..14].try_into().ok()?) as usize;
        let width = i32::from_le_bytes(bytes[18..22].try_into().ok()?) as u32;
        let height = i32::from_le_bytes(bytes[22..26].try_into().ok()?) as u32;
        let frame = Self {
            width,
            height,
            stride: width as usize * 3,
            pixels: bytes.get(offset..)?.to_vec(),
        };
        Some(frame)
    }

    fn pixel(&self, column: u32, row: u32) -> [u8; 3] {
        let offset = (self.height as usize - 1 - row as usize) * self.stride + column as usize * 3;
        [
            self.pixels[offset + 2],
            self.pixels[offset + 1],
            self.pixels[offset],
        ]
    }
}

#[test]
fn fisheye_routes_require_a_session() {
    let client = client();
    assert_eq!(
        client
            .get("/api/fisheye/map?mode=equirect&width=64&height=32")
            .dispatch()
            .status(),
        Status::Unauthorized
    );
    assert_eq!(
        client
            .get("/api/fisheye/dewarp?mode=equirect&width=64&height=32")
            .dispatch()
            .status(),
        Status::Unauthorized
    );
}

#[test]
fn model_names_the_recovered_functions() {
    let client = client();
    let response = client.get("/api/fisheye/model").dispatch();
    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&response.into_string().unwrap()).unwrap();
    assert_eq!(body["library"], "libfisheye.so");
    assert_eq!(body["projected"]["radius"], 240.0);
    assert_eq!(body["projected"]["centerX"], 256.0);
    assert_eq!(body["projected"]["maxThetaDeg"], 90.0);
    assert_eq!(body["lensTypes"].as_array().unwrap().len(), 8);
    let mounts = body["mountTypes"].as_array().unwrap();
    assert_eq!(mounts.len(), 3);
    assert_eq!(mounts[0]["name"], "ceiling");
    assert_eq!(mounts[0]["pitchDeg"], 0.0);
    assert_eq!(mounts[1]["pitchDeg"], 180.0);
    assert_eq!(mounts[2]["pitchDeg"], 90.0);
    let functions: Vec<&str> = body["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert!(functions.contains(&"CFisheyeMap::GetThetaXYFromFlatXY"));
    assert!(functions.contains(&"Fisheye_GenAndSetLensInfo"));
}

#[test]
fn equirect_map_covers_the_hemisphere() {
    let client = client();
    sign_in(&client);
    let response = client
        .get("/api/fisheye/map?mode=equirect&width=256&height=128")
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&response.into_string().unwrap()).unwrap();
    assert_eq!(body["width"], 256);
    assert_eq!(body["height"], 128);
    assert_eq!(body["valid"], 16_384);
    let oversize = client
        .get("/api/fisheye/map?mode=equirect&width=720&height=360")
        .dispatch();
    assert_eq!(oversize.status(), Status::PayloadTooLarge);
}

#[test]
fn ptz_map_keeps_every_centre_pixel() {
    let client = client();
    sign_in(&client);
    let response = client
        .get("/api/fisheye/map?mode=ptz&width=320&height=180&pan=30&tilt=0&fov=70")
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&response.into_string().unwrap()).unwrap();
    let valid = body["valid"].as_u64().unwrap();
    assert_eq!(valid, 320 * 180);
}

#[test]
fn mount_rotates_the_covered_hemisphere() {
    let client = client();
    sign_in(&client);
    let ceiling: serde_json::Value = serde_json::from_str(
        &client
            .get("/api/fisheye/map?mode=equirect&width=64&height=32&mount=ceiling")
            .dispatch()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    let table: serde_json::Value = serde_json::from_str(
        &client
            .get("/api/fisheye/map?mode=equirect&width=64&height=32&mount=table")
            .dispatch()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    let wall: serde_json::Value = serde_json::from_str(
        &client
            .get("/api/fisheye/map?mode=equirect&width=64&height=32&mount=wall")
            .dispatch()
            .into_string()
            .unwrap(),
    )
    .unwrap();
    let ceiling_sources = ceiling["sources"].as_array().unwrap();
    let table_sources = table["sources"].as_array().unwrap();
    let wall_sources = wall["sources"].as_array().unwrap();
    assert_ne!(ceiling_sources, table_sources);
    assert_ne!(ceiling_sources, wall_sources);
    assert_ne!(table_sources, wall_sources);
    let ceiling_centre = &ceiling_sources[16 * 64 + 32];
    assert_ne!(ceiling_centre[0], -1.0);
    let table_centre = &table_sources[16 * 64 + 32];
    assert_eq!(table_centre[0], -1.0);
    let unknown = client
        .get("/api/fisheye/map?mode=equirect&width=64&height=32&mount=floor")
        .dispatch();
    assert_eq!(unknown.status(), Status::BadRequest);
}

#[test]
fn dewarp_returns_a_bitmap_with_the_fixture_spokes_on_their_columns() {
    let client = client();
    sign_in(&client);
    let response = client
        .get("/api/fisheye/dewarp?mode=equirect&width=720&height=360")
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.content_type().map(|value| value.to_string()),
        Some("image/bmp".to_string())
    );
    let bytes = response.into_bytes().expect("bitmap body");
    let bitmap = Bitmap::parse(&bytes).expect("bitmap header");
    assert_eq!(bitmap.width, 720);
    assert_eq!(bitmap.height, 360);

    let mut worst = 0.0f32;
    for index in 0..12usize {
        let reference = spoke_color(index);
        let mut columns = Vec::new();
        for row in 198..315 {
            for column in 0..720 {
                let pixel = bitmap.pixel(column, row);
                if (pixel[0] as i32 - reference[0] as i32).abs() <= 8
                    && (pixel[1] as i32 - reference[1] as i32).abs() <= 8
                    && (pixel[2] as i32 - reference[2] as i32).abs() <= 8
                {
                    columns.push(column as f32);
                }
            }
        }
        assert!(columns.len() > 60, "spoke {index} pixels {}", columns.len());
        columns.sort_by(|left, right| left.partial_cmp(right).unwrap());
        let measured = (0.25 - (columns[columns.len() / 2] + 0.5) / 720.0) * 360.0;
        let expected = 15.0 + index as f32 * 30.0;
        let mut delta = (measured - expected).abs() % 360.0;
        if delta > 180.0 {
            delta = 360.0 - delta;
        }
        worst = worst.max(delta);
    }
    assert!(worst < 1.0, "worst bearing error {worst}");
}

#[test]
fn fisheye_script_is_served() {
    let client = client();
    let response = client.get("/fisheye.js").dispatch();
    assert_eq!(response.status(), Status::Ok);
    let body = response.into_string().unwrap();
    assert!(body.contains("ProviewFisheye"));
    assert!(body.contains("webgl2"));
}
