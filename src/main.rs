#[macro_use]
extern crate rocket;

use tracing_subscriber::EnvFilter;

#[launch]
fn rocket() -> _ {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    proview_cctv_web::app::build(rocket::Config::figment())
}
