//! Connection workflow shared by the GUI and the headless CLI:
//! directory config -> leaf config -> privileged core.

use nthlink_common::leafconf::{self, BuiltConfig, LeafOptions};
use nthlink_common::model::DirectoryServerConfig;
use nthlink_common::paths;
use nthlink_common::rootgo::Bridge;
use nthlink_common::state::ClientState;
use nthlink_common::tunnel::CoreEvent;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;

pub struct Prepared {
    pub directory: DirectoryServerConfig,
    pub leaf: BuiltConfig,
    pub from_cache: bool,
    pub config_path: PathBuf,
    pub log_path: PathBuf,
}

/// Fetch the directory config (falling back to the last good one), persist
/// `data`, and write the leaf config for the core.
pub fn prepare(state: &mut ClientState, progress: &dyn Fn(&str)) -> Result<Prepared, String> {
    let bridge = Bridge::locate()?;
    if !bridge.prefix_exists() {
        progress("Preparing the Wine environment (first run, takes a moment)…");
    } else {
        progress("Contacting the nthLink directory service…");
    }
    let param = state.config_parameter();
    let (directory, from_cache) = match bridge.get_config(&state.data, &param, 3) {
        Ok(cfg) if cfg.is_usable() || cfg.service_unavailable().is_some() => {
            ClientState::save_last_config(&cfg);
            (cfg, false)
        }
        Ok(_) => match ClientState::load_last_config() {
            Some(c) => (c, true),
            None => return Err("The directory service returned no usable servers".into()),
        },
        Err(e) => match ClientState::load_last_config() {
            Some(c) if c.is_usable() => {
                progress("Directory service unreachable, using the last known servers…");
                (c, true)
            }
            _ => return Err(format!("Could not get servers: {e}")),
        },
    };
    if let Some(d) = directory.data.as_deref().filter(|d| !d.is_empty()) {
        state.update_data(d);
    }
    if let Some(sa) = directory.service_unavailable() {
        let msg = match sa.altern.as_ref().filter(|_| sa.has_altern) {
            Some(a) if !a.app_name.is_empty() => format!(
                "This service is not available in your current region. Please use {} instead. {}",
                a.app_name, a.website
            ),
            _ => "This service is not available in your current region. Please try using a different VPN application.".into(),
        };
        return Err(msg);
    }

    let cache = paths::cache_dir();
    paths::ensure_private_dir(&cache).map_err(|e| e.to_string())?;
    paths::ensure_private_dir(&paths::log_dir()).map_err(|e| e.to_string())?;
    let log_path = paths::log_dir().join("leaf.log");
    let opts = LeafOptions {
        log_level: state.settings.log_level.clone(),
        log_file: Some(log_path.to_string_lossy().into_owned()),
    };
    let leaf = leafconf::build(&directory, &opts)?;
    let config_path = cache.join("leaf.conf");
    paths::write_private(&config_path, leaf.text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(Prepared { directory, leaf, from_cache, config_path, log_path })
}

/// The core binary as root can execute it. Inside an AppImage the bundle is a
/// FUSE mount that root cannot read, so the binary is staged into the user's
/// cache directory first.
fn core_binary() -> Result<PathBuf, String> {
    let res = paths::resource_dir().ok_or("Bundled resources not found")?;
    let src = res.join("nthlink-core");
    if !src.is_file() {
        return Err(format!("Missing {}", src.display()));
    }
    let in_appimage = std::env::var_os("APPIMAGE").is_some() || src.starts_with("/tmp/.mount_");
    if !in_appimage {
        return Ok(src);
    }
    let dir = paths::cache_dir().join("bin");
    paths::ensure_private_dir(&dir).map_err(|e| e.to_string())?;
    let dst = dir.join("nthlink-core");
    let same = std::fs::read(&src).ok() == std::fs::read(&dst).ok();
    if !same {
        let tmp = dir.join("nthlink-core.partial");
        std::fs::copy(&src, &tmp).map_err(|e| format!("stage core: {e}"))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    }
    Ok(dst)
}

pub enum Elevation {
    /// Already root.
    None,
    /// Graphical polkit prompt.
    Pkexec,
    /// Terminal password prompt.
    Sudo,
}

pub struct CoreHandle {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl CoreHandle {
    /// Ask the core to stop (closing stdin is equivalent).
    pub fn stop(&mut self) {
        if let Some(mut s) = self.stdin.take() {
            let _ = writeln!(s, "stop");
        }
    }

    pub fn wait(&mut self) -> Option<i32> {
        self.stdin.take();
        self.child.wait().ok().and_then(|s| s.code())
    }
}

/// Events from the core, plus the process exit.
pub enum CoreMsg {
    Event(CoreEvent),
    /// stdout closed (process exiting); carries what it printed on stderr.
    Exited(Option<i32>, String),
    /// Sent by the owner of the channel to ask the supervising thread to stop.
    StopRequested,
}

/// Drive a started core until it exits: forwards events through `forward`,
/// handles `StopRequested`, reaps the process, and finally forwards
/// `Exited(exit_code, stderr)`.
pub fn supervise(mut handle: CoreHandle, rx: mpsc::Receiver<CoreMsg>, forward: impl Fn(CoreMsg)) {
    while let Ok(msg) = rx.recv() {
        match msg {
            CoreMsg::StopRequested => handle.stop(),
            CoreMsg::Exited(_, err) => {
                let code = handle.wait();
                forward(CoreMsg::Exited(code, err));
                return;
            }
            ev => forward(ev),
        }
    }
    let code = handle.wait();
    forward(CoreMsg::Exited(code, String::new()));
}

/// How traffic is captured.
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    /// System-wide TUN (needs root).
    Tun,
    /// Local SOCKS5/HTTP proxy on loopback (no root).
    Proxy { socks: u16, http: u16 },
}

pub fn spawn_core(prep: &Prepared, mode: Mode, elevation: Elevation, tx: mpsc::Sender<CoreMsg>) -> Result<CoreHandle, String> {
    let core = match mode {
        Mode::Tun => core_binary()?,
        // Runs as the user, straight from the bundle.
        Mode::Proxy { .. } => paths::resource_dir().ok_or("Bundled resources not found")?.join("nthlink-core"),
    };
    let mut cmd = match elevation {
        Elevation::None => Command::new(&core),
        Elevation::Pkexec => {
            let mut c = Command::new("pkexec");
            c.arg(&core);
            c
        }
        Elevation::Sudo => {
            let mut c = Command::new("sudo");
            c.arg("--").arg(&core);
            c
        }
    };
    match mode {
        Mode::Tun => {
            cmd.arg("run");
        }
        Mode::Proxy { socks, http } => {
            cmd.arg("proxy").arg("--socks-port").arg(socks.to_string()).arg("--http-port").arg(http.to_string());
        }
    }
    cmd.arg("--config").arg(&prep.config_path).arg("--log").arg(&prep.log_path);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start the tunnel helper: {e}"))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let err_tx = tx.clone();
    let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sb = stderr_buf.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Ok(mut b) = sb.lock() {
                b.push_str(&line);
                b.push('\n');
            }
            let _ = err_tx.send(CoreMsg::Event(CoreEvent::Log { level: "stderr".into(), message: line }));
        }
    });
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(ev) = serde_json::from_str::<CoreEvent>(&line) {
                if tx.send(CoreMsg::Event(ev)).is_err() {
                    return;
                }
            }
        }
        // stdout closed: the process is exiting. The exit code is reaped by
        // the owner of the handle; report what stderr said.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let err = stderr_buf.lock().map(|b| b.clone()).unwrap_or_default();
        let _ = tx.send(CoreMsg::Exited(None, err));
    });
    Ok(CoreHandle { child, stdin })
}

/// Human message for pkexec's own exit codes.
pub fn explain_exit(code: Option<i32>, stderr: &str) -> Option<String> {
    match code {
        Some(126) => Some("Authorization was cancelled".into()),
        Some(127) if stderr.contains("No authentication agent") || stderr.is_empty() => {
            Some("Administrator rights are required, but no polkit authentication agent is running.".into())
        }
        Some(127) => Some(format!("Not authorized: {}", stderr.trim())),
        _ => None,
    }
}

/// End-to-end check through the tunnel (the Windows client's health check
/// pings through leaf as well). Plain HTTP so it doesn't need a TLS stack.
pub fn health_check() -> bool {
    use std::io::Read;
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let Ok(mut addrs) = ("connectivitycheck.gstatic.com", 80).to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(10)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    if s
        .write_all(b"GET /generate_204 HTTP/1.1\r\nHost: connectivitycheck.gstatic.com\r\nConnection: close\r\nUser-Agent: nthLink\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 64];
    match s.read(&mut buf) {
        Ok(n) if n > 12 => {
            let head = String::from_utf8_lossy(&buf[..n]);
            head.starts_with("HTTP/1.1 204") || head.starts_with("HTTP/1.0 204")
        }
        _ => false,
    }
}

pub fn open_url(url: &str) {
    if url.is_empty() {
        return;
    }
    let _ = Command::new("xdg-open").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}
