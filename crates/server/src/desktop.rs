use std::sync::OnceLock;

use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::State;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub const TOKEN_HEADER: &str = "x-proview-desktop-token";
pub const INDEX: &str = include_str!("../../../web/index.html");

pub struct DesktopControl {
    token: String,
    authority: String,
    requests: UnboundedSender<()>,
    page: OnceLock<&'static str>,
}

impl DesktopControl {
    pub fn new(token: impl Into<String>) -> (Self, UnboundedReceiver<()>) {
        let (requests, receiver) = mpsc::unbounded_channel();
        (
            Self {
                token: token.into(),
                authority: String::new(),
                requests,
                page: OnceLock::new(),
            },
            receiver,
        )
    }

    pub fn for_authority(mut self, authority: impl Into<String>) -> Self {
        self.authority = authority.into();
        self
    }

    pub fn serves(&self, host: &str) -> bool {
        !self.authority.is_empty() && matches(host, &self.authority)
    }

    pub fn page(&self) -> &'static str {
        self.page.get_or_init(|| {
            let page = INDEX.replace("</head>", &format!("{}\n</head>", self.bootstrap()));
            Box::leak(page.into_boxed_str())
        })
    }

    fn bootstrap(&self) -> String {
        let token = serde_json::to_string(&self.token).unwrap_or_else(|_| "\"\"".to_string());
        format!("<script>window.__PROVIEW_DESKTOP__ = {{ token: {token} }};</script>")
    }
}

pub struct PageHost(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for PageHost {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let host = request.headers().get_one("Host").unwrap_or_default();
        Outcome::Success(PageHost(host.to_string()))
    }
}

pub struct OfferedToken(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for OfferedToken {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let offered = request.headers().get_one(TOKEN_HEADER).unwrap_or_default();
        Outcome::Success(OfferedToken(offered.to_string()))
    }
}

fn matches(offered: &str, expected: &str) -> bool {
    let offered = offered.as_bytes();
    let expected = expected.as_bytes();
    if offered.len() != expected.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in offered.iter().zip(expected) {
        difference |= left ^ right;
    }
    difference == 0
}

#[post("/desktop/restart")]
fn restart(
    control: &State<Option<DesktopControl>>,
    host: PageHost,
    offered: OfferedToken,
) -> Status {
    let Some(control) = control.inner().as_ref() else {
        return Status::NotFound;
    };
    if !control.serves(&host.0) {
        return Status::Forbidden;
    }
    if !matches(&offered.0, &control.token) {
        return Status::Forbidden;
    }
    match control.requests.send(()) {
        Ok(()) => Status::Accepted,
        Err(_) => Status::ServiceUnavailable,
    }
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![restart]
}
