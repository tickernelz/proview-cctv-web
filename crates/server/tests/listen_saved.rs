use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Running {
    child: Option<Child>,
    dir: PathBuf,
}

impl Running {
    fn start(tag: &str, saved: &str, environment: Option<&str>) -> Self {
        let dir = std::env::temp_dir().join(format!("proview-listen-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("data dir");
        std::fs::write(
            dir.join("settings.json"),
            format!(r#"{{"values":{{"PROVIEW_LISTEN":"{saved}"}}}}"#),
        )
        .expect("settings.json");
        let mut command = Command::new(env!("CARGO_BIN_EXE_proview-server"));
        command
            .env_remove("PROVIEW_LISTEN")
            .env("PROVIEW_DATA_DIR", &dir)
            .env("PROVIEW_PROVIDER", "fixture")
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(value) = environment {
            command.env("PROVIEW_LISTEN", value);
        }
        let child = command.spawn().expect("proview-server");
        Self {
            child: Some(child),
            dir,
        }
    }

    fn stop(mut self) -> String {
        let mut child = self.child.take().expect("child");
        let _ = child.kill();
        let _ = child.wait();
        let mut log = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_string(&mut log);
        }
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut log);
        }
        log
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("free port")
        .local_addr()
        .expect("address")
        .port()
}

fn health(port: u16, within: Duration) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let started = Instant::now();
    while started.elapsed() < within {
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let request = format!("GET /api/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
            if stream.write_all(request.as_bytes()).is_ok() {
                let mut head = [0u8; 12];
                if stream.read_exact(&mut head).is_ok() && head.starts_with(b"HTTP/1.") {
                    return &head[9..12] == b"200";
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn refuses(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(300),
    )
    .is_err()
}

#[test]
fn the_server_listens_on_a_loopback_address_saved_in_settings() {
    let port = free_port();
    let server = Running::start("loopback", &format!("127.0.0.1:{port}"), None);
    let answered = health(port, Duration::from_secs(30));
    let log = server.stop();
    assert!(
        answered,
        "nothing answered on the saved address 127.0.0.1:{port}: {log}"
    );
    assert!(log.contains("listen address chosen"), "{log}");
    assert!(
        log.contains("settings.json"),
        "the log names the source: {log}"
    );
}

#[test]
fn a_saved_network_address_is_ignored_and_the_environment_address_is_used() {
    let saved = free_port();
    let chosen = free_port();
    let server = Running::start(
        "network",
        &format!("0.0.0.0:{saved}"),
        Some(&format!("127.0.0.1:{chosen}")),
    );
    let answered = health(chosen, Duration::from_secs(30));
    let closed = refuses(saved);
    let log = server.stop();
    assert!(
        answered,
        "nothing answered on the environment address: {log}"
    );
    assert!(closed, "the saved 0.0.0.0:{saved} was bound: {log}");
    assert!(
        log.contains("not loopback"),
        "the ignored value is logged: {log}"
    );
    assert!(
        log.contains("settings.json"),
        "the warning names the source: {log}"
    );
    assert!(
        log.contains("PROVIEW_LISTEN"),
        "the chosen source is logged: {log}"
    );
}
