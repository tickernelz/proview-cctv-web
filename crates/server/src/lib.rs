#[macro_use]
extern crate rocket;

pub mod app;
pub mod live_state;
pub mod routes;

pub use app::{build, build_with, serve, ServeOptions};
