use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;

use crate::config::ModeStore;
use proview_core::config::Config;
use proview_core::session::Store;
use rocket::figment::Figment;
use rocket::Shutdown;

pub struct ServeOptions {
    pub address: String,
    pub port: u16,
    pub secret_key: Option<String>,
    pub data_dir: Option<PathBuf>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8787,
            secret_key: None,
            data_dir: None,
        }
    }
}

pub struct ServerHandle {
    pub address: SocketAddr,
    pub shutdown: Shutdown,
}

pub fn figment(opts: &ServeOptions, port: u16) -> Figment {
    let mut figment = Figment::from(rocket::Config::figment())
        .merge(("address", opts.address.clone()))
        .merge(("port", port))
        .merge(("log_level", "critical"));

    if let Some(key) = &opts.secret_key {
        figment = figment.merge(("secret_key", key.clone()));
    }
    figment
}

pub fn build(figment: Figment) -> rocket::Rocket<rocket::Build> {
    build_with(figment, Config::from_env())
}

pub fn build_with(figment: Figment, config: Config) -> rocket::Rocket<rocket::Build> {
    build_with_modes(figment, config, ModeStore::from_env(None))
}

pub fn build_with_modes(
    figment: Figment,
    config: Config,
    modes: ModeStore,
) -> rocket::Rocket<rocket::Build> {
    build_with_state(
        figment,
        config,
        modes,
        crate::settings::SettingsStore::load(None),
    )
}

pub fn build_with_state(
    figment: Figment,
    config: Config,
    modes: ModeStore,
    settings: crate::settings::SettingsStore,
) -> rocket::Rocket<rocket::Build> {
    rocket::custom(figment)
        .manage(config)
        .manage(Store::new())
        .manage(modes)
        .manage(crate::live_state::LiveState::new())
        .manage(crate::routes::cloud::CloudState::new())
        .manage(crate::routes::cloud_bind::AccountStore::from_env())
        .manage(settings)
        .mount("/", crate::routes::files())
        .mount("/api", crate::routes::api())
        .mount("/api", crate::config::routes())
        .mount("/api", crate::lens_route::routes())
        .mount("/api", crate::routes::cloud::api())
        .mount("/api", crate::settings::routes())
        .mount("/", crate::routes::live())
}

pub async fn serve(opts: ServeOptions) -> Result<ServerHandle, Box<rocket::Error>> {
    let settings = crate::settings::SettingsStore::load(opts.data_dir.as_deref());
    settings.apply_env();
    serve_with_state(opts, Config::from_env(), settings).await
}

pub async fn serve_with(
    opts: ServeOptions,
    config: Config,
) -> Result<ServerHandle, Box<rocket::Error>> {
    let settings = crate::settings::SettingsStore::load(opts.data_dir.as_deref());
    serve_with_state(opts, config, settings).await
}

pub async fn serve_with_state(
    opts: ServeOptions,
    config: Config,
    settings: crate::settings::SettingsStore,
) -> Result<ServerHandle, Box<rocket::Error>> {
    let port = reserve_port(&opts.address, opts.port);
    if let Some(dir) = &opts.data_dir {
        std::env::set_var("PROVIEW_DATA_DIR", dir);
    }
    let modes = ModeStore::from_env(opts.data_dir.as_deref());
    let rig = build_with_state(figment(&opts, port), config, modes, settings);
    let server = rig.ignite().await.map_err(Box::new)?;
    let shutdown = server.shutdown();
    let address = SocketAddr::new(
        opts.address
            .parse()
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        port,
    );
    tokio::spawn(async move {
        if let Err(error) = server.launch().await {
            tracing::error!(%error, "server stopped unexpectedly");
        }
    });
    Ok(ServerHandle { address, shutdown })
}

fn reserve_port(address: &str, requested: u16) -> u16 {
    if requested != 0 {
        return requested;
    }
    match TcpListener::bind((address, 0)) {
        Ok(listener) => listener.local_addr().map(|a| a.port()).unwrap_or(0),
        Err(_) => 0,
    }
}
