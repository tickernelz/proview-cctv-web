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
    build_with_state_and_accounts(
        figment,
        config,
        modes,
        settings,
        crate::routes::cloud_bind::AccountStore::from_env(),
    )
}

pub fn build_with_state_and_accounts(
    figment: Figment,
    config: Config,
    modes: ModeStore,
    settings: crate::settings::SettingsStore,
    accounts: crate::routes::cloud_bind::AccountStore,
) -> rocket::Rocket<rocket::Build> {
    build_with_control(
        figment,
        config,
        modes,
        settings,
        accounts,
        None,
        Store::new(),
    )
}

pub(crate) fn build_with_control(
    figment: Figment,
    config: Config,
    modes: ModeStore,
    settings: crate::settings::SettingsStore,
    accounts: crate::routes::cloud_bind::AccountStore,
    control: Option<crate::desktop::DesktopControl>,
    sessions: Store,
) -> rocket::Rocket<rocket::Build> {
    let cloud = std::sync::Arc::new(proview_core::provider::cloud::CloudProvider::new(&config));
    rocket::custom(figment)
        .manage(config)
        .manage(cloud)
        .manage(sessions)
        .manage(modes)
        .manage(crate::live_state::LiveState::new())
        .manage(crate::routes::cloud::CloudState::new())
        .manage(accounts)
        .manage(settings)
        .manage(control)
        .mount("/", crate::routes::files())
        .mount("/api", crate::desktop::routes())
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
    serve_shared(opts, config, settings)
        .await
        .map(|(handle, _)| handle)
}

pub async fn serve_desktop(
    opts: ServeOptions,
    config: Config,
    settings: crate::settings::SettingsStore,
    control: crate::desktop::DesktopControl,
) -> Result<ServerHandle, Box<rocket::Error>> {
    let (handle, _) = serve_desktop_shared(opts, config, settings, control).await?;
    Ok(handle)
}

pub async fn serve_desktop_shared(
    opts: ServeOptions,
    config: Config,
    settings: crate::settings::SettingsStore,
    control: crate::desktop::DesktopControl,
) -> Result<(ServerHandle, Store), Box<rocket::Error>> {
    serve_with_control(opts, config, settings, Some(control)).await
}

pub async fn serve_shared(
    opts: ServeOptions,
    config: Config,
    settings: crate::settings::SettingsStore,
) -> Result<(ServerHandle, Store), Box<rocket::Error>> {
    serve_with_control(opts, config, settings, None).await
}

async fn serve_with_control(
    opts: ServeOptions,
    config: Config,
    settings: crate::settings::SettingsStore,
    control: Option<crate::desktop::DesktopControl>,
) -> Result<(ServerHandle, Store), Box<rocket::Error>> {
    let port = reserve_port(&opts.address, opts.port);
    let modes = ModeStore::from_env(opts.data_dir.as_deref());
    let accounts = crate::routes::cloud_bind::AccountStore::load(opts.data_dir.as_deref());
    let control = control.map(|control| control.for_authority(format!("{}:{port}", opts.address)));
    let sessions = Store::new();
    let rig = build_with_control(
        figment(&opts, port),
        config,
        modes,
        settings,
        accounts,
        control,
        sessions.clone(),
    );
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
    Ok((ServerHandle { address, shutdown }, sessions))
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
