//! nthlink-core: the privileged half of the nthLink Linux client.
//!
//! It plays the role the NetFilter driver + leaf.dll play on Windows: run leaf
//! with a TUN inbound and steer the machine's traffic into it.
//!
//! Routing is done wg-quick style so it is safe by construction:
//!   * leaf marks its own sockets (`LEAF_SOCKET_MARK`) and those use `main`;
//!   * `main` is consulted first with the default route suppressed, so LAN and
//!     other specific routes keep working;
//!   * everything else hits a table whose only route is `default dev nthlink0`.
//! The kernel deletes that route together with the TUN device, so if leaf (or
//! this process) dies for any reason the machine falls back to its normal
//! routing instead of losing connectivity. The system default route is never
//! touched.
//!
//! Usage:
//!   nthlink-core run   --config <leaf.conf> [--log <file>] [--dns auto|resolved|resolvconf|none]   (root)
//!   nthlink-core proxy --config <leaf.conf> [--log <file>] [--socks-port N] [--http-port N]
//!   nthlink-core check --config <leaf.conf>
//!   nthlink-core cleanup                                                                      (root)
//!
//! `run` prints one JSON event per line (see `nthlink_common::tunnel::CoreEvent`)
//! and stops when stdin reaches EOF or receives "stop", or on SIGINT/SIGTERM/SIGHUP.

mod dns;
mod net;

use nthlink_common::tunnel::{self, CoreEvent};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const STATE_DIR: &str = "/run/nthlink";
const RT_ID: u16 = 0;

static SIGNALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    SIGNALLED.store(true, Ordering::SeqCst);
}

pub fn emit(ev: CoreEvent) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", ev.to_line());
    let _ = out.flush();
}

pub fn log(level: &str, message: impl Into<String>) {
    emit(CoreEvent::Log { level: level.into(), message: message.into() });
}

fn usage() -> ! {
    eprintln!(
        "usage: nthlink-core run   --config <leaf.conf> [--log <file>] [--dns auto|resolved|resolvconf|none]\n       nthlink-core proxy --config <leaf.conf> [--log <file>] [--socks-port N] [--http-port N]\n       nthlink-core check --config <leaf.conf>\n       nthlink-core cleanup"
    );
    std::process::exit(64);
}

fn main() {
    // pkexec/sudo give us a minimal environment; be explicit about PATH.
    std::env::set_var("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        for s in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            libc::signal(s, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        }
    }
    // leaf holds sockets per connection; the inherited soft limit (1024) is
    // far too low for a whole-system tunnel.
    nthlink_common::rlimit::raise_nofile();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut config = None;
    let mut log_file = None;
    let mut dns_choice = "auto".to_string();
    let mut socks_port: u16 = 1080;
    let mut http_port: u16 = 1087;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().unwrap_or_else(|| usage());
        match a.as_str() {
            "--config" => config = Some(val()),
            "--log" => log_file = Some(val()),
            "--dns" => dns_choice = val(),
            "--socks-port" => socks_port = val().parse().unwrap_or_else(|_| usage()),
            "--http-port" => http_port = val().parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }
    let needs_root = matches!(args.first().map(String::as_str), Some("run") | Some("cleanup"));
    if needs_root && unsafe { libc::geteuid() } != 0 {
        emit(CoreEvent::Error { message: "nthlink-core must run as root".into() });
        std::process::exit(77);
    }
    let result = match args.first().map(String::as_str) {
        Some("cleanup") => {
            net::cleanup();
            dns::restore();
            std::process::exit(0);
        }
        Some("check") => {
            let path = config.unwrap_or_else(|| usage());
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| leaf::config::from_string(&t).map_err(|e| e.to_string())) {
                Ok(c) => {
                    println!("ok: {} inbound(s), {} outbound(s), {} rule(s)", c.inbounds.len(), c.outbounds.len(), c.router.as_ref().map(|r| r.rules.len()).unwrap_or(0));
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("invalid config: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some("run") => run(&config.unwrap_or_else(|| usage()), log_file.as_deref(), &dns_choice),
        Some("proxy") => proxy(&config.unwrap_or_else(|| usage()), log_file.as_deref(), socks_port, http_port),
        _ => usage(),
    };
    let code = match result {
        Ok(()) => 0,
        Err(e) => {
            emit(CoreEvent::Error { message: e });
            1
        }
    };
    emit(CoreEvent::Stopped);
    std::process::exit(code);
}

/// uid of the desktop user who asked for elevation.
fn invoking_uid() -> Option<u32> {
    ["PKEXEC_UID", "SUDO_UID"].iter().find_map(|k| std::env::var(k).ok()?.parse().ok())
}

/// Make sure the log file exists and belongs to the desktop user, so the
/// GUI can read and rotate it.
fn prepare_log(path: &str) {
    use std::os::unix::fs::OpenOptionsExt;
    let p = std::path::Path::new(path);
    if p.is_file() && p.metadata().map(|m| m.len() > 4 * 1024 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(p, p.with_extension("log.1"));
    }
    let _ = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(p);
    if let Some(uid) = invoking_uid() {
        if let Ok(c) = std::ffi::CString::new(path) {
            unsafe {
                libc::chown(c.as_ptr(), uid, u32::MAX);
            }
        }
    }
}

fn start_leaf(config: String) -> Result<std::thread::JoinHandle<Result<(), String>>, String> {
    std::thread::Builder::new()
        .name("leaf".into())
        .spawn(move || {
            leaf::start(
                RT_ID,
                leaf::StartOptions {
                    config: leaf::Config::Str(config),
                    runtime_opt: leaf::RuntimeOption::MultiThreadAuto(2 * 1024 * 1024),
                },
            )
            .map_err(|e| format!("leaf: {e}"))
        })
        .map_err(|e| e.to_string())
}

/// stdin EOF (GUI exited or closed the pipe) or "stop" ends the session.
fn watch_stdin() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) if l.trim() == "stop" => break,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        flag.store(true, Ordering::SeqCst);
    });
    stop
}

fn stop_requested(stop: &AtomicBool) -> bool {
    stop.load(Ordering::SeqCst) || SIGNALLED.load(Ordering::SeqCst)
}

/// Local SOCKS5/HTTP proxy only: no TUN, no routing, no root.
fn proxy(config_path: &str, log_file: Option<&str>, socks_port: u16, http_port: u16) -> Result<(), String> {
    emit(CoreEvent::Starting);
    let text = std::fs::read_to_string(config_path).map_err(|e| format!("read {config_path}: {e}"))?;
    let config = nthlink_common::leafconf::to_proxy_mode(&text, socks_port, http_port);
    let config_for_check = config.clone();
    if let Some(l) = log_file {
        prepare_log(l);
    }
    let leaf_thread = start_leaf(config)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if leaf_thread.is_finished() {
            let r = leaf_thread.join().unwrap_or_else(|_| Err("leaf panicked".into()));
            return Err(match r {
                Err(e) => format!("Failed to start the proxy: {e}"),
                Ok(()) => "The proxy stopped during startup".into(),
            });
        }
        if leaf::is_running(RT_ID) && std::net::TcpStream::connect(("127.0.0.1", socks_port)).is_ok() {
            break;
        }
        if Instant::now() > deadline || SIGNALLED.load(Ordering::SeqCst) {
            stop_leaf(leaf_thread);
            return Err(format!("The proxy did not start listening on 127.0.0.1:{socks_port}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    log("info", "checking servers…");
    if !warm_up(&config_for_check, None) {
        log("warn", "No server answered yet; leaf keeps retrying in the background");
    }
    emit(CoreEvent::Connected { ipv6: false, dns: format!("local proxy (SOCKS5 127.0.0.1:{socks_port}, HTTP 127.0.0.1:{http_port})") });
    let stop = watch_stdin();
    let mut result = Ok(());
    while !stop_requested(&stop) {
        if leaf_thread.is_finished() {
            result = Err("The proxy stopped unexpectedly".to_string());
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    stop_leaf(leaf_thread);
    result
}

fn run(config_path: &str, log_file: Option<&str>, dns_choice: &str) -> Result<(), String> {
    emit(CoreEvent::Starting);
    let _ = std::fs::create_dir_all(STATE_DIR);
    let _lock = lock_instance()?;

    // Leftovers from a crashed session would conflict with what we add below.
    net::cleanup();
    dns::restore();

    let config = std::fs::read_to_string(config_path).map_err(|e| format!("read {config_path}: {e}"))?;
    if let Some(l) = log_file {
        prepare_log(l);
    }

    std::env::set_var("LEAF_SOCKET_MARK", tunnel::FWMARK.to_string());
    let config_for_check = config.clone();
    let leaf_thread = start_leaf(config)?;

    // Wait for leaf to bring the TUN up.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if leaf_thread.is_finished() {
            let r = leaf_thread.join().unwrap_or_else(|_| Err("leaf panicked".into()));
            net::cleanup();
            return Err(match r {
                Err(e) => format!("Failed to start the tunnel: {e}"),
                Ok(()) => "The tunnel stopped during startup".into(),
            });
        }
        if net::tun_exists() && leaf::is_running(RT_ID) {
            break;
        }
        if Instant::now() > deadline || SIGNALLED.load(Ordering::SeqCst) {
            stop_leaf(leaf_thread);
            net::cleanup();
            return Err("Timed out waiting for the TUN device".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // Only move the system's traffic over once a server is known to work.
    log("info", "checking servers…");
    if !warm_up(&config_for_check, None) {
        if SIGNALLED.load(Ordering::SeqCst) {
            stop_leaf(leaf_thread);
            net::cleanup();
            return Ok(());
        }
        log("warn", "No server answered yet; leaf keeps retrying in the background");
    }

    let ipv6 = match net::setup() {
        Ok(v6) => v6,
        Err(e) => {
            stop_leaf(leaf_thread);
            net::cleanup();
            return Err(format!("Routing setup failed: {e}"));
        }
    };
    let dns_mode = dns::setup(dns_choice);
    if !ipv6 {
        log("warn", "IPv6 could not be routed into the tunnel on this system");
    }
    emit(CoreEvent::Connected { ipv6, dns: dns_mode.describe().into() });

    let stop = watch_stdin();
    let mut result = Ok(());
    let mut last_watch = Instant::now();
    loop {
        if stop_requested(&stop) {
            break;
        }
        if leaf_thread.is_finished() {
            result = Err("The tunnel stopped unexpectedly".to_string());
            break;
        }
        if !net::tun_exists() {
            result = Err("The TUN device disappeared".to_string());
            break;
        }
        if last_watch.elapsed() > Duration::from_secs(3) {
            last_watch = Instant::now();
            // NetworkManager & co. may rewrite resolv.conf or flush rules on
            // network changes; put ours back.
            if let Err(e) = net::ensure() {
                log("warn", format!("re-applying routes: {e}"));
            }
            dns::ensure(&dns_mode);
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    dns::restore();
    stop_leaf(leaf_thread);
    net::cleanup();
    result
}

/// One tunnel at a time: a second core would tear down the first one's TUN.
fn lock_instance() -> Result<std::fs::File, String> {
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(format!("{STATE_DIR}/core.lock"))
        .map_err(|e| format!("lock: {e}"))?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Another nthLink tunnel is already running".into());
    }
    Ok(f)
}

/// Probe the default outbound until it works, like the Windows client's
/// HealthCheckMonitor (`leaf_health_check(rt, "Proxy", timeout)`). This also
/// warms up multiplexed connections so the first real requests don't stall.
fn warm_up(config: &str, stop: Option<&AtomicBool>) -> bool {
    let Some(tag) = nthlink_common::leafconf::final_outbound(config) else {
        return true;
    };
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return true;
    };
    for attempt in 1..=6 {
        if SIGNALLED.load(Ordering::SeqCst) || stop.map(|s| s.load(Ordering::SeqCst)).unwrap_or(false) {
            return false;
        }
        let manager = leaf::RUNTIME_MANAGER.lock().unwrap().get(&RT_ID).cloned();
        let Some(m) = manager else { return false };
        let ok = rt
            .block_on(m.health_check_outbound(&tag, Some(Duration::from_secs(10))))
            .map(|(tcp, _udp)| tcp.is_ok())
            .unwrap_or(false);
        if ok {
            return true;
        }
        log("info", format!("server check {attempt}/6 failed, retrying…"));
        std::thread::sleep(Duration::from_secs(2));
    }
    false
}

fn stop_leaf(t: std::thread::JoinHandle<Result<(), String>>) {
    if leaf::is_running(RT_ID) {
        leaf::shutdown(RT_ID);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !t.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if t.is_finished() {
        let _ = t.join();
    }
}
