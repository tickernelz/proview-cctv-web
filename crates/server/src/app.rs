use std::net::{SocketAddr, TcpListener};

use proview_core::config::Config;
use proview_core::session::Store;
use rocket::figment::Figment;
use rocket::Shutdown;

pub struct ServeOptions {
    pub address: String,
    pub port: u16,
    pub secret_key: Option<String>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8787,
            secret_key: None,
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
    rocket::custom(figment)
        .manage(config)
        .manage(Store::new())
        .manage(crate::live_state::LiveState::new())
        .mount("/", crate::routes::files())
        .mount("/api", crate::routes::api())
        .mount("/", crate::routes::live())
}

pub async fn serve(opts: ServeOptions) -> Result<ServerHandle, Box<rocket::Error>> {
    let port = reserve_port(&opts.address, opts.port);
    let rig = build(figment(&opts, port));
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
