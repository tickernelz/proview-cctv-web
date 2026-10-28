use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::State;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub const TOKEN_HEADER: &str = "x-proview-desktop-token";

pub struct DesktopControl {
    token: String,
    requests: UnboundedSender<()>,
}

impl DesktopControl {
    pub fn new(token: impl Into<String>) -> (Self, UnboundedReceiver<()>) {
        let (requests, receiver) = mpsc::unbounded_channel();
        (
            Self {
                token: token.into(),
                requests,
            },
            receiver,
        )
    }
}

struct OfferedToken(String);

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
fn restart(control: &State<Option<DesktopControl>>, offered: OfferedToken) -> Status {
    let Some(control) = control.inner().as_ref() else {
        return Status::NotFound;
    };
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

pub fn bootstrap(control: &DesktopControl) -> String {
    let token = serde_json::to_string(&control.token).unwrap_or_else(|_| "\"\"".to_string());
    format!("<script>window.__PROVIEW_DESKTOP__ = {{ token: {token} }};</script>")
}
