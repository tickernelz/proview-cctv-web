use proview_server::ServeOptions;
use tracing_subscriber::EnvFilter;

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

    let listen = argument("--listen")
        .or_else(|| std::env::var("PROVIEW_LISTEN").ok())
        .unwrap_or_else(|| "127.0.0.1:8787".to_string());
    let (address, port) = listen
        .rsplit_once(':')
        .map(|(host, port)| (host.to_string(), port.parse().unwrap_or(8787)))
        .unwrap_or_else(|| ("127.0.0.1".to_string(), 8787));

    let opts = ServeOptions {
        address,
        port,
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
    use super::flag;

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
}
