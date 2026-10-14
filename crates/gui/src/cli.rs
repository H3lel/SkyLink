//! Headless mode: `nthlink connect` runs the whole workflow in a terminal.

use crate::engine::{self, CoreMsg, Elevation, Mode};
use nthlink_common::state::ClientState;
use nthlink_common::tunnel::CoreEvent;
use std::io::IsTerminal;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn elevation() -> Elevation {
    if unsafe { libc::geteuid() } == 0 {
        Elevation::None
    } else if std::io::stdin().is_terminal() {
        Elevation::Sudo
    } else {
        Elevation::Pkexec
    }
}

pub fn connect() -> i32 {
    session(Mode::Tun)
}

/// `nthlink proxy [socks-port [http-port]]`: local proxy only, no root.
pub fn proxy(args: &[String]) -> i32 {
    let socks = args.first().and_then(|a| a.parse().ok()).unwrap_or(1080);
    let http = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(1087);
    session(Mode::Proxy { socks, http })
}

fn session(mode: Mode) -> i32 {
    // Ctrl+C reaches the core through the terminal's process group; we stay
    // alive until it has cleaned up.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    let mut state = ClientState::load();
    let prep = match engine::prepare(&mut state, &|m| eprintln!("… {m}")) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    eprintln!(
        "… using {}{}; leaf log: {}",
        prep.leaf.summary,
        if prep.from_cache { " (cached)" } else { "" },
        prep.log_path.display()
    );
    let (tx, rx) = mpsc::channel();
    let elev = match mode {
        Mode::Tun => elevation(),
        Mode::Proxy { .. } => Elevation::None,
    };
    let mut handle = match engine::spawn_core(&prep, mode, elev, tx) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let mut connected = false;
    let mut last_check = Instant::now();
    let mut stderr = String::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(CoreMsg::Event(CoreEvent::Connected { ipv6, dns })) => {
                connected = true;
                last_check = Instant::now() - Duration::from_secs(55);
                match mode {
                    Mode::Tun => eprintln!("✓ Connected (DNS via {dns}, IPv6 {}). Press Ctrl+C to disconnect.", if ipv6 { "tunneled" } else { "not tunneled" }),
                    Mode::Proxy { .. } => eprintln!("✓ Proxy ready: {dns}. Press Ctrl+C to stop."),
                }
                if state.settings.open_landing_page && matches!(mode, Mode::Tun) {
                    engine::open_url(&prep.directory.redirect_url);
                }
            }
            Ok(CoreMsg::Event(CoreEvent::Error { message })) => eprintln!("error: {message}"),
            Ok(CoreMsg::Event(CoreEvent::Log { level, message })) => {
                if level == "stderr" {
                    stderr.push_str(&message);
                    stderr.push('\n');
                }
                eprintln!("[{level}] {message}");
            }
            Ok(CoreMsg::Event(CoreEvent::Stopped)) => eprintln!("Disconnected."),
            Ok(CoreMsg::Event(CoreEvent::Starting)) => eprintln!("… starting tunnel"),
            Ok(CoreMsg::StopRequested) => {}
            Ok(CoreMsg::Exited(..)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if connected && matches!(mode, Mode::Tun) && last_check.elapsed() > Duration::from_secs(60) {
            last_check = Instant::now();
            std::thread::spawn(|| {
                if !engine::health_check() {
                    eprintln!("! health check failed (the tunnel may be degraded)");
                }
            });
        }
    }
    let code = handle.wait();
    if let Some(msg) = engine::explain_exit(code, &stderr) {
        eprintln!("error: {msg}");
    }
    code.unwrap_or(1)
}

/// `nthlink fetch`: show what the directory service returns (passwords hidden).
pub fn fetch() -> i32 {
    let mut state = ClientState::load();
    let bridge = match nthlink_common::rootgo::Bridge::locate() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match bridge.get_config(&state.data, &state.config_parameter(), 1) {
        Ok(cfg) => {
            if let Some(d) = cfg.data.as_deref().filter(|d| !d.is_empty()) {
                state.update_data(d);
            }
            ClientState::save_last_config(&cfg);
            println!("custom config: {}", cfg.use_custom_config);
            for s in &cfg.servers {
                println!("server: {} {}:{} ws={}", s.protocol, s.host, s.port, s.ws);
            }
            println!("landing page: {}", cfg.redirect_url);
            println!("news items: {}", cfg.headline_news.len());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `nthlink cleanup`: remove routes/DNS left by a killed tunnel.
pub fn cleanup() -> i32 {
    let res = match nthlink_common::paths::resource_dir() {
        Some(r) => r.join("nthlink-core"),
        None => {
            eprintln!("error: resources not found");
            return 1;
        }
    };
    let status = match elevation() {
        Elevation::None => std::process::Command::new(&res).arg("cleanup").status(),
        Elevation::Sudo => std::process::Command::new("sudo").arg("--").arg(&res).arg("cleanup").status(),
        Elevation::Pkexec => std::process::Command::new("pkexec").arg(&res).arg("cleanup").status(),
    };
    status.ok().and_then(|s| s.code()).unwrap_or(1)
}

/// `nthlink render [directory.json]`: print the generated leaf config.
pub fn render(path: Option<String>) -> i32 {
    let cfg = match path {
        Some(p) => std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())),
        None => ClientState::load_last_config().ok_or_else(|| "no cached directory config".to_string()),
    };
    let cfg: nthlink_common::model::DirectoryServerConfig = match cfg {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let state = ClientState::load();
    let opts = nthlink_common::leafconf::LeafOptions { log_level: state.settings.log_level.clone(), log_file: None };
    match nthlink_common::leafconf::build(&cfg, &opts) {
        Ok(b) => {
            print!("{}", b.text);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
