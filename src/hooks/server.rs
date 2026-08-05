//! Loopback report server: receives hook reports over HTTP and forwards them
//! to the app on an mpsc channel (drained in `ui()` like the PTY channel).
//!
//! Security note: the server binds 127.0.0.1 with no auth token. Safety
//! relies on (a) loopback-only binding, (b) the port file living in the
//! user's config dir with mode 0600, and (c) same-user access — any process
//! running as this user could in principle forge reports. See
//! docs/phase3-hooks.md for the full discussion.

use super::{HookReport, parse_report};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tiny_http::{Method, Response, Server, StatusCode};

pub const PORT_FILE_NAME: &str = "agentmux.port";

/// The report server plus the app-side receiver.
///
/// The port file is written at startup and removed on drop (clean exit).
pub struct ReportServer {
    pub port: u16,
    /// App-side channel end; drain it each frame.
    pub receiver: Receiver<HookReport>,
    port_file: PathBuf,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ReportServer {
    /// Bind the loopback server on an ephemeral port and write the port
    /// file to the default config location. Fails only if binding or
    /// writing the port file fails.
    pub fn start() -> std::io::Result<Self> {
        Self::start_with_port_file(Self::port_file_path()?)
    }

    /// Bind the loopback server on an ephemeral port, writing the port file
    /// to `port_file` (used by tests to isolate per-test files; the product
    /// uses [`Self::start`]).
    pub fn start_with_port_file(port_file: PathBuf) -> std::io::Result<Self> {
        let server = Server::http("127.0.0.1:0").map_err(std::io::Error::other)?;
        let port = server.server_addr().to_ip().map(|addr| addr.port()).unwrap_or(0);
        if let Some(dir) = port_file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_port_file(&port_file, port)?;

        let (sender, receiver) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown);
        let thread = std::thread::Builder::new()
            .name("agentmux-hook-server".to_owned())
            .spawn(move || serve(server, sender, shutdown_flag))
            .map_err(std::io::Error::other)?;

        Ok(Self {
            port,
            receiver,
            port_file,
            shutdown,
            thread: Some(thread),
        })
    }

    /// The port file path hooks read to find the server.
    pub fn port_file_path() -> std::io::Result<PathBuf> {
        let base = dirs::config_dir()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory"))?;
        Ok(base.join("agentmux").join(PORT_FILE_NAME))
    }
}

impl Drop for ReportServer {
    fn drop(&mut self) {
        // Ask the accept loop to exit, join the thread, remove the port file.
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.port_file);
    }
}

fn serve(server: Server, sender: Sender<HookReport>, shutdown: Arc<AtomicBool>) {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        match server.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(mut request)) => {
                let url = request.url().to_owned();
                let method = request.method().clone();
                let status = handle(&mut request.as_reader(), &url, &method, &sender);
                let _ = request.respond(Response::empty(status));
            }
            Ok(None) => {}
            Err(_) => break, // listener closed
        }
    }
}

fn handle(
    reader: &mut dyn Read,
    url: &str,
    method: &Method,
    sender: &Sender<HookReport>,
) -> StatusCode {
    if method != &Method::Post {
        return StatusCode(405);
    }
    if url != "/report" {
        return StatusCode(404);
    }
    let mut body = Vec::new();
    if reader.read_to_end(&mut body).is_err() {
        return StatusCode(400);
    }
    let report = match parse_report(&String::from_utf8_lossy(&body)) {
        Ok(report) => report,
        Err(_) => return StatusCode(400),
    };
    if sender.send(report).is_err() {
        return StatusCode(500);
    }
    StatusCode(200)
}

/// Write the port file with mode 0600 on Unix (private to the user).
fn write_port_file(path: &std::path::Path, port: u16) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        let mut file = options.open(path)?;
        file.write_all(port.to_string().as_bytes())?;
        file.sync_all()
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, port.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookState;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Unique port-file path per call so parallel tests never share one.
    static PORT_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_port_file() -> PathBuf {
        let n = PORT_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "agentmux-portfile-test-{}-{n}.port",
            std::process::id()
        ))
    }

    #[test]
    fn port_file_roundtrip_and_cleanup() {
        let path = unique_port_file();
        let server = ReportServer::start_with_port_file(path.clone()).expect("server should start");
        assert!(server.port > 0);
        let port = std::fs::read_to_string(&path).expect("port file should exist");
        assert_eq!(port.parse::<u16>().unwrap(), server.port);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "port file must be user-private");
        }
        drop(server);
        assert!(!path.exists(), "port file must be removed on drop");
    }

    #[test]
    fn server_rejects_bad_reports() {
        // Start a server (with an isolated port file) and send raw HTTP to
        // it; only valid reports pass.
        let path = unique_port_file();
        let server = ReportServer::start_with_port_file(path).unwrap();
        let port = server.port;

        let post = |body: &str| -> u16 {
            let client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            use std::io::Write;
            let mut stream = client;
            write!(
                stream,
                "POST /report HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            let mut response = String::new();
            let mut reader = std::io::BufReader::new(stream.take(512));
            reader.read_to_string(&mut response).unwrap();
            response
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|code| code.parse().ok())
                .unwrap_or(0)
        };

        assert_eq!(post("garbage"), 400);
        assert_eq!(post(r#"{"pid": 1, "agent": "cortana", "state": "working"}"#), 400);
        assert_eq!(post(r#"{"pid": 1, "agent": "claude", "state": "working"}"#), 200);
        assert_eq!(post(r#"{"pid": 2, "agent": "omp", "state": "blocked", "message": "m"}"#), 200);

        let report = server.receiver.try_recv().unwrap();
        assert_eq!(report.pid, 1);
        assert_eq!(report.state, HookState::Working);
        let report = server.receiver.try_recv().unwrap();
        assert_eq!(report.pid, 2);
        assert_eq!(report.state, HookState::Blocked);
        assert_eq!(report.message.as_deref(), Some("m"));
    }
}
