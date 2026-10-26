#[macro_use]
extern crate rocket;

mod config;
mod provider;
mod routes;
mod session;

use tracing_subscriber::EnvFilter;

#[launch]
fn rocket() -> _ {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    rocket::build()
        .manage(config::Config::from_env())
        .manage(session::Store::new())
        .mount("/", routes::files())
        .mount("/api", routes::api())
}
