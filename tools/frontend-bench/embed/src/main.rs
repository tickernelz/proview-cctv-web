#[macro_use]
extern crate rocket;

use std::borrow::Cow;
use std::io::Cursor;
use std::path::PathBuf;

use rocket::http::ContentType;
use rocket::request::Request;
use rocket::response::{self, Responder, Response};

struct Asset {
    body: Cow<'static, [u8]>,
    kind: ContentType,
    gzip: bool,
}

impl<'r> Responder<'r, 'static> for Asset {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut res = Response::build();
        res.header(self.kind)
            .sized_body(self.body.len(), Cursor::new(self.body));
        if self.gzip {
            res.raw_header("Content-Encoding", "gzip");
        }
        res.ok()
    }
}

fn kind(path: &str) -> ContentType {
    match path.rsplit('.').next() {
        Some("html") => ContentType::HTML,
        Some("js") => ContentType::JavaScript,
        Some("css") => ContentType::CSS,
        Some("woff2") => ContentType::WOFF2,
        _ => ContentType::Plain,
    }
}

#[cfg(feature = "per-route")]
const FILES: &[(&str, &[u8])] = &[
    ("index.html", include_bytes!("../../../../web/index.html")),
    ("app.js", include_bytes!("../../../../web/app.js")),
    ("fisheye.js", include_bytes!("../../../../web/fisheye.js")),
    ("grid.js", include_bytes!("../../../../web/grid.js")),
    ("account.js", include_bytes!("../../../../web/account.js")),
    ("talk.js", include_bytes!("../../../../web/talk.js")),
    (
        "recordings.js",
        include_bytes!("../../../../web/recordings.js"),
    ),
    (
        "device-admin.js",
        include_bytes!("../../../../web/device-admin.js"),
    ),
    ("alarms.js", include_bytes!("../../../../web/alarms.js")),
    ("playback.js", include_bytes!("../../../../web/playback.js")),
    (
        "device-settings.js",
        include_bytes!("../../../../web/device-settings.js"),
    ),
    ("style.css", include_bytes!("../../../../web/style.css")),
    ("tokens.css", include_bytes!("../../../../web/tokens.css")),
    (
        "fonts/plex-sans-400.woff2",
        include_bytes!("../../../../web/fonts/plex-sans-400.woff2"),
    ),
    (
        "fonts/plex-sans-500.woff2",
        include_bytes!("../../../../web/fonts/plex-sans-500.woff2"),
    ),
    (
        "fonts/plex-sans-600.woff2",
        include_bytes!("../../../../web/fonts/plex-sans-600.woff2"),
    ),
    (
        "fonts/plex-mono-400.woff2",
        include_bytes!("../../../../web/fonts/plex-mono-400.woff2"),
    ),
    (
        "fonts/plex-mono-500.woff2",
        include_bytes!("../../../../web/fonts/plex-mono-500.woff2"),
    ),
    (
        "fonts/OFL.txt",
        include_bytes!("../../../../web/fonts/OFL.txt"),
    ),
];

#[cfg(feature = "per-route")]
fn lookup(path: &str) -> Option<Asset> {
    FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(name, bytes)| Asset {
            body: Cow::Borrowed(bytes),
            kind: kind(name),
            gzip: false,
        })
}

#[cfg(feature = "embedded-dir")]
#[derive(rust_embed::Embed)]
#[folder = "../../../web"]
struct Web;

#[cfg(feature = "embedded-dir")]
fn lookup(path: &str) -> Option<Asset> {
    Web::get(path).map(|file| Asset {
        body: file.data,
        kind: kind(path),
        gzip: false,
    })
}

#[cfg(feature = "embedded-gz")]
#[derive(rust_embed::Embed)]
#[folder = "gz"]
struct Web;

#[cfg(feature = "embedded-gz")]
fn lookup(path: &str) -> Option<Asset> {
    Web::get(&format!("{path}.gz")).map(|file| Asset {
        body: file.data,
        kind: kind(path),
        gzip: true,
    })
}

#[cfg(not(any(
    feature = "per-route",
    feature = "embedded-dir",
    feature = "embedded-gz"
)))]
fn lookup(path: &str) -> Option<Asset> {
    (path == "index.html").then(|| Asset {
        body: Cow::Borrowed(b"<!doctype html>"),
        kind: kind(path),
        gzip: false,
    })
}

#[get("/")]
fn index() -> Option<Asset> {
    lookup("index.html")
}

#[get("/<path..>", rank = 20)]
fn asset(path: PathBuf) -> Option<Asset> {
    lookup(path.to_str()?)
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![index, asset])
}
