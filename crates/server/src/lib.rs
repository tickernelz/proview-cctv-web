#[macro_use]
extern crate rocket;

pub mod app;
pub mod routes;

pub use app::{build, serve, ServeOptions};
