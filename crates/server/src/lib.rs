#[macro_use]
extern crate rocket;

pub mod app;
pub mod config;
pub mod desktop;
pub mod desktop_alarms;
pub mod lens_route;
pub mod live_state;
pub mod routes;
pub mod settings;
pub mod stream;

pub use app::{
    build, build_with, build_with_modes, build_with_state, build_with_state_and_accounts,
    choose_port, serve, serve_desktop, serve_desktop_shared, serve_shared, serve_with,
    serve_with_state, PortChoice, ServeOptions,
};
