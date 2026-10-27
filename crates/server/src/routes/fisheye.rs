use proview_core::fisheye::{Calibration, Curve, Lens, LensType, Map, MountType, RgbFrame, View};
use rocket::form::FromForm;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

use std::sync::OnceLock;

use proview_core::session::Store;

const FIXTURE_BYTES: &[u8] = include_bytes!("../../../../fixtures/fisheye/fisheye-grid-512.ppm");
const FIXTURE_WIDTH: u32 = 512;
const FIXTURE_HEIGHT: u32 = 512;
const FIXTURE_CENTER: f32 = 256.0;
const FIXTURE_RADIUS: f32 = 240.0;
const MAX_MAP_PIXELS: u32 = 65_536;
const MAX_RENDER_PIXELS: u32 = 4_194_304;

pub fn api() -> Vec<rocket::route::Route> {
    routes![model, map, dewarp]
}

#[derive(Serialize)]
pub struct FisheyeErrorBody {
    pub ok: bool,
    pub message: String,
}

type Failure = (Status, Json<FisheyeErrorBody>);

fn fail(status: Status, message: impl Into<String>) -> Failure {
    (
        status,
        Json(FisheyeErrorBody {
            ok: false,
            message: message.into(),
        }),
    )
}

fn session_id(cookies: &CookieJar<'_>) -> Result<String, Failure> {
    cookies
        .get_private("pvw_session")
        .map(|cookie| cookie.value().to_string())
        .ok_or_else(|| fail(Status::Unauthorized, "not signed in"))
}

fn signed_in(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<(), Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .map(|_| ())
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceEntry {
    pub name: &'static str,
    pub address: &'static str,
    pub role: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeTableEntry {
    pub name: &'static str,
    pub code: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub library: &'static str,
    pub library_sha256: &'static str,
    pub projection: &'static str,
    pub functions: Vec<ProvenanceEntry>,
    pub mount_types: Vec<CodeTableEntry>,
    pub lens_types: Vec<CodeTableEntry>,
    pub fixture: Calibration,
    pub projected: FixtureCalibration,
    pub defaults: View,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureCalibration {
    pub image_width: u32,
    pub image_height: u32,
    pub center_x: f32,
    pub center_y: f32,
    pub radius: f32,
    pub max_theta_deg: f32,
}

#[get("/fisheye/model")]
fn model() -> Json<ModelResponse> {
    Json(ModelResponse {
        library: "libfisheye.so",
        library_sha256: "5ab93e63b4917d60379eb866db84a1453b460ab8caf54738dda01c0b0ffff7ad",
        projection: "equidistant sphere, pole at the lens axis, equirectangular output with the vendor bearing/elevation convention",
        functions: vec![
            ProvenanceEntry {
                name: "CFisheyeMap::GetThetaXYFromFlatXY",
                address: "0x103d0",
                role: "radius to polar angle, bearing and elevation split",
            },
            ProvenanceEntry {
                name: "Fisheye_GenAndSetLensInfo",
                address: "0x71a0",
                role: "radial calibration table, scale 2*xd_max/pi",
            },
            ProvenanceEntry {
                name: "Fisheye_PanTiltToInVPicturePoint",
                address: "0x6278",
                role: "virtual pan, tilt, zoom to fisheye pixel",
            },
            ProvenanceEntry {
                name: "FisheyeGL_UpdatePanoramaShaderParameters",
                address: "0xa4c4",
                role: "shader rotation matrix and view uniforms",
            },
            ProvenanceEntry {
                name: "glsl Transform / GetSourcePoint",
                address: "0x134e8",
                role: "unified sphere view ray, equirectangular source lookup",
            },
            ProvenanceEntry {
                name: "Fisheye_InternalSetOption",
                address: "0x69dc",
                role: "option mask, mount 0x40, dewarp type 0x80",
            },
        ],
        mount_types: mount_table(),
        lens_types: lens_table(),
        fixture: fixture_calibration(),
        projected: project_fixture(),
        defaults: View::default(),
    })
}

fn mount_table() -> Vec<CodeTableEntry> {
    (0..=2u8)
        .filter_map(|code| {
            MountType::from_code(code).map(|mount| CodeTableEntry {
                name: match mount {
                    MountType::Ceiling => "ceiling",
                    MountType::Table => "table",
                    MountType::Wall => "wall",
                },
                code: mount.code(),
            })
        })
        .collect()
}

fn lens_table() -> Vec<CodeTableEntry> {
    (0..=7u8)
        .filter_map(|code| {
            LensType::from_code(code).map(|lens| CodeTableEntry {
                name: match lens {
                    LensType::General => "general",
                    LensType::Vr360 => "vr360",
                    LensType::Lvr360 => "lvr360",
                    LensType::Vr180 => "vr180",
                    LensType::DualVr360 => "dualVr360",
                    LensType::DualVr180 => "dualVr180",
                    LensType::Vr180Pro => "vr180Pro",
                    LensType::SinglePip => "singlePip",
                },
                code: lens.code(),
            })
        })
        .collect()
}

fn fixture_calibration() -> Calibration {
    Calibration {
        lens_type: LensType::Vr180.code(),
        center_offset_x: FIXTURE_CENTER as i16 - FIXTURE_WIDTH as i16 / 2,
        center_offset_y: FIXTURE_CENTER as i16 - FIXTURE_HEIGHT as i16 / 2,
        radius: FIXTURE_RADIUS as i16,
        image_width: FIXTURE_WIDTH as i16,
        image_height: FIXTURE_HEIGHT as i16,
        view_angle: 180,
        view_mode: 1,
    }
}

fn fixture_lens() -> Lens {
    Lens {
        width: FIXTURE_WIDTH,
        height: FIXTURE_HEIGHT,
        center_x: FIXTURE_CENTER,
        center_y: FIXTURE_CENTER,
        radius: FIXTURE_RADIUS,
        curve: Curve::Equidistant {
            max_theta_deg: 90.0,
        },
    }
}

fn project_fixture() -> FixtureCalibration {
    let lens = fixture_lens();
    FixtureCalibration {
        image_width: lens.width,
        image_height: lens.height,
        center_x: lens.center_x,
        center_y: lens.center_y,
        radius: lens.radius,
        max_theta_deg: lens.curve.max_theta().to_degrees(),
    }
}

#[derive(FromForm, Clone)]
pub struct MapQuery {
    mode: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    radius: Option<f32>,
    #[field(name = "centerX")]
    center_x: Option<f32>,
    #[field(name = "centerY")]
    center_y: Option<f32>,
    #[field(name = "maxTheta")]
    max_theta: Option<f32>,
    pan: Option<f32>,
    tilt: Option<f32>,
    fov: Option<f32>,
    #[field(name = "camZ")]
    cam_z: Option<f32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapResponse {
    pub width: u32,
    pub height: u32,
    pub valid: usize,
    pub sources: Vec<[f32; 2]>,
}

fn build_lens(query: &MapQuery) -> Result<Lens, Failure> {
    let max_theta = query.max_theta.unwrap_or(90.0);
    let curve = Curve::equidistant(max_theta)
        .ok_or_else(|| fail(Status::BadRequest, "maxTheta must be between 0 and 180"))?;
    Lens::new(
        FIXTURE_WIDTH,
        FIXTURE_HEIGHT,
        query.center_x.unwrap_or(FIXTURE_CENTER),
        query.center_y.unwrap_or(FIXTURE_CENTER),
        query.radius.unwrap_or(FIXTURE_RADIUS),
    )
    .map(|lens| lens.with_curve(curve))
    .ok_or_else(|| fail(Status::BadRequest, "radius and image size must be positive"))
}

fn build_view(query: &MapQuery) -> View {
    View {
        pan_deg: query.pan.unwrap_or(0.0),
        tilt_deg: query.tilt.unwrap_or(0.0),
        fov_deg: query.fov.unwrap_or(90.0),
        cam_z: query.cam_z.unwrap_or(0.0),
    }
}

fn build_map_for(lens: &Lens, query: &MapQuery, limit: u32) -> Result<Map, Failure> {
    let width = query.width.unwrap_or(640).clamp(1, 4096);
    let height = query.height.unwrap_or(320).clamp(1, 4096);
    if width * height > limit {
        return Err(fail(
            Status::PayloadTooLarge,
            format!("map above {limit} pixels"),
        ));
    }
    match query.mode.as_deref().unwrap_or("equirect") {
        "equirect" => Ok(Map::equirect(lens, width, height)),
        "ptz" => Ok(Map::ptz(lens, width, height, &build_view(query))),
        other => Err(fail(
            Status::BadRequest,
            format!("unknown mode {other}, expected equirect or ptz"),
        )),
    }
}

#[get("/fisheye/map?<query..>")]
async fn map(
    query: MapQuery,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<MapResponse>, Failure> {
    signed_in(store, cookies)?;
    let map = build_map(&query, MAX_MAP_PIXELS).await?;
    let valid = map.valid_count();
    let sources = map
        .sources
        .iter()
        .map(|entry| match entry {
            Some((x, y)) => [*x, *y],
            None => [-1.0, -1.0],
        })
        .collect();
    Ok(Json(MapResponse {
        width: map.width,
        height: map.height,
        valid,
        sources,
    }))
}

#[get("/fisheye/dewarp?<query..>")]
async fn dewarp(
    query: MapQuery,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(ContentType, Vec<u8>), Failure> {
    signed_in(store, cookies)?;
    let render = rocket::tokio::task::spawn_blocking(move || {
        let map = build_map_blocking(&query, MAX_RENDER_PIXELS)?;
        let frame = fixture_frame()?;
        let warped = frame.warp(&map);
        Ok::<_, Failure>(encode_bmp(&warped))
    })
    .await
    .map_err(|error| fail(Status::InternalServerError, error.to_string()))??;
    Ok((ContentType::new("image", "bmp"), render))
}

async fn build_map(query: &MapQuery, limit: u32) -> Result<Map, Failure> {
    let query = query.clone();
    rocket::tokio::task::spawn_blocking(move || build_map_blocking(&query, limit))
        .await
        .map_err(|error| fail(Status::InternalServerError, error.to_string()))?
}

fn build_map_blocking(query: &MapQuery, limit: u32) -> Result<Map, Failure> {
    let lens = build_lens(query)?;
    build_map_for(&lens, query, limit)
}

fn fixture_frame() -> Result<&'static RgbFrame, Failure> {
    static FRAME: OnceLock<Option<RgbFrame>> = OnceLock::new();
    FRAME
        .get_or_init(|| RgbFrame::from_ppm(FIXTURE_BYTES))
        .as_ref()
        .ok_or_else(|| fail(Status::InternalServerError, "embedded fixture is not a binary PPM"))
}

fn encode_bmp(frame: &RgbFrame) -> Vec<u8> {
    let row_bytes = frame.width as usize * 3;
    let padding = (4 - row_bytes % 4) % 4;
    let stride = row_bytes + padding;
    let image_bytes = stride * frame.height as usize;
    let file_bytes = 54 + image_bytes;
    let mut out = Vec::with_capacity(file_bytes);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_bytes as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(frame.width as i32).to_le_bytes());
    out.extend_from_slice(&(frame.height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(image_bytes as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let pad = [0u8; 4];
    for row in (0..frame.height).rev() {
        for column in 0..frame.width {
            let pixel = frame.pixel(column, row);
            out.push(pixel[2]);
            out.push(pixel[1]);
            out.push(pixel[0]);
        }
        out.extend_from_slice(&pad[..padding]);
    }
    out
}
