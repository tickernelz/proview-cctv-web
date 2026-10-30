use std::net::SocketAddr;

use proview_server::settings::{SettingsStore, LISTEN_KEY};
use proview_server::ServeOptions;
use tracing_subscriber::EnvFilter;

const DEFAULT_LISTEN: &str = "127.0.0.1:8787";
const SAVED_SOURCE: &str = "settings.json";

fn flag<S: AsRef<str>>(args: impl IntoIterator<Item = S>, name: &str) -> bool {
    args.into_iter().any(|item| item.as_ref() == name)
}

fn argument(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(item) = args.next() {
        if item == name {
            return args.next();
        }
        if let Some(rest) = item.strip_prefix(&format!("{name}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

fn filled(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn listen_address(
    flag: Option<String>,
    environment: Option<String>,
    saved: Option<String>,
) -> Result<(SocketAddr, &'static str), String> {
    let saved = filled(saved).and_then(|value| match value.parse::<SocketAddr>() {
        Ok(address) if address.ip().to_canonical().is_loopback() => Some(address),
        Ok(address) => {
            tracing::warn!(
                %address,
                source = SAVED_SOURCE,
                "ignoring the saved listen address because it is not loopback; pass --listen or set PROVIEW_LISTEN to listen on the network"
            );
            None
        }
        Err(_) => {
            tracing::warn!(
                value = %value,
                source = SAVED_SOURCE,
                "ignoring the saved listen address because it is not an IP address and port"
            );
            None
        }
    });
    for (value, source) in [(flag, "--listen"), (environment, "PROVIEW_LISTEN")] {
        if let Some(value) = filled(value) {
            return value.parse().map(|address| (address, source)).map_err(|_| {
                format!(
                    "{source} is {value:?}, not an IP address and port such as {DEFAULT_LISTEN}"
                )
            });
        }
    }
    Ok(match saved {
        Some(address) => (address, SAVED_SOURCE),
        None => (
            DEFAULT_LISTEN.parse().expect("default listen address"),
            "default",
        ),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    if flag(std::env::args().skip(1), "--version") || flag(std::env::args().skip(1), "-V") {
        println!("proview-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let saved = SettingsStore::load(None);
    let (listen, source) = listen_address(
        argument("--listen"),
        std::env::var(LISTEN_KEY).ok(),
        saved.value(LISTEN_KEY),
    )?;
    tracing::info!(address = %listen, source, "listen address chosen");

    let opts = ServeOptions {
        address: listen.ip().to_string(),
        port: listen.port(),
        secret_key: std::env::var("ROCKET_SECRET_KEY").ok(),
        data_dir: None,
    };
    let handle = proview_server::serve(opts).await?;
    tracing::info!(address = %handle.address, "proview-server listening");
    shutdown_signal().await?;
    handle.shutdown.notify();
    Ok(())
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[cfg(test)]
mod tests {
    use super::{flag, listen_address};

    #[test]
    fn a_bare_flag_needs_no_value_after_it() {
        assert!(flag(["--version"], "--version"));
        assert!(flag(["-V"], "-V"));
        assert!(flag(
            ["--listen", "127.0.0.1:8787", "--version"],
            "--version"
        ));
        assert!(!flag(["--listen", "127.0.0.1:8787"], "--version"));
    }

    fn some(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    fn chosen(
        flag: Option<String>,
        environment: Option<String>,
        saved: Option<String>,
    ) -> (String, &'static str) {
        let (address, source) = listen_address(flag, environment, saved).expect("an address");
        (address.to_string(), source)
    }

    #[test]
    fn the_flag_then_the_environment_then_a_loopback_saved_value_choose_the_address() {
        let saved = some("127.0.0.1:9911");
        assert_eq!(
            chosen(None, None, saved.clone()),
            ("127.0.0.1:9911".to_string(), "settings.json")
        );
        assert_eq!(
            chosen(None, None, some("[::1]:9912")),
            ("[::1]:9912".to_string(), "settings.json")
        );
        assert_eq!(
            chosen(None, some("0.0.0.0:8800"), saved.clone()),
            ("0.0.0.0:8800".to_string(), "PROVIEW_LISTEN")
        );
        assert_eq!(
            chosen(some("127.0.0.1:7700"), some("0.0.0.0:8800"), saved),
            ("127.0.0.1:7700".to_string(), "--listen")
        );
        assert_eq!(
            chosen(None, some(" "), None),
            ("127.0.0.1:8787".to_string(), "default")
        );
    }

    #[test]
    fn a_saved_network_or_unparsable_address_is_ignored() {
        for saved in [
            "0.0.0.0:8800",
            "192.168.1.5:8800",
            "[::]:8800",
            "localhost:8800",
            "nope",
        ] {
            assert_eq!(
                chosen(None, None, some(saved)),
                ("127.0.0.1:8787".to_string(), "default"),
                "{saved}"
            );
            assert_eq!(
                chosen(None, some("127.0.0.1:9000"), some(saved)),
                ("127.0.0.1:9000".to_string(), "PROVIEW_LISTEN"),
                "{saved}"
            );
        }
    }

    #[test]
    fn an_unparsable_flag_or_environment_value_stops_the_start() {
        let error = listen_address(some("localhost:8787"), None, None).expect_err("flag");
        assert!(error.contains("--listen"), "{error}");
        let error = listen_address(None, some("nope"), None).expect_err("environment");
        assert!(error.contains("PROVIEW_LISTEN"), "{error}");
    }
}
