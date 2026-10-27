#[macro_use]
extern crate rocket;

pub mod app;
pub mod config;
pub mod lens_route;
pub mod live_state;
pub mod routes;
pub mod settings;
pub mod stream;

pub use app::{
    build, build_with, build_with_modes, build_with_state, serve, serve_with, serve_with_state,
    ServeOptions,
};
