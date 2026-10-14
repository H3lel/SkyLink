//! Runs the vendor's directory-service client (rootGo.dll) through Wine via
//! `rootgo-bridge.exe`. The DLL is a Go c-shared library with no Linux build;
//! it is headless and only needs Wine's kernel32/ws2_32/crypt32, so it runs
//! reliably in a private, display-less Wine prefix.

use crate::model::{ConfigParameter, DirectoryServerConfig, FeedbackParameter};
use crate::paths;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const BRIDGE_EXE: &str = "rootgo-bridge.exe";
const ROOTGO_DLL: &str = "rootGo.dll";
const APIKEY: &str = "apikey";
/// Folder inside the prefix's C: drive the bridge is staged into, so no Wine
/// path ever points into the (FUSE-mounted) AppImage.
const STAGE: &str = "nthlink";

static CALL_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct Bridge {
    wine: PathBuf,
    wineserver: Option<PathBuf>,
    prefix: PathBuf,
    resources: PathBuf,
}

#[derive(Debug)]
pub struct CallResult {
    pub output: String,
    /// JSON objects passed to the library's failure callback.
    pub failures: Vec<String>,
    /// Everything else the library logged.
    pub log: String,
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

impl Bridge {
    pub fn locate() -> Result<Self, String> {
        let resources = paths::resource_dir()
            .ok_or_else(|| "Bundled resources (rootGo.dll) not found; set NTHLINK_RESOURCES".to_string())?;
        for f in [BRIDGE_EXE, ROOTGO_DLL, APIKEY] {
            if !resources.join(f).is_file() {
                return Err(format!("Missing bundled resource {}", resources.join(f).display()));
            }
        }
        let wine = std::env::var_os("NTHLINK_WINE")
            .map(PathBuf::from)
            .or_else(|| which("wine"))
            .or_else(|| which("wine64"))
            .ok_or_else(|| {
                "Wine is required to talk to the nthLink directory service.\nInstall it with:  sudo apt install wine   (or your distribution's equivalent)".to_string()
            })?;
        let wineserver = wine.parent().map(|d| d.join("wineserver")).filter(|p| p.is_file()).or_else(|| which("wineserver"));
        Ok(Self { wine, wineserver, prefix: paths::data_dir().join("wineprefix"), resources })
    }

    pub fn prefix_exists(&self) -> bool {
        self.prefix.join("system.reg").is_file()
    }

    fn command(&self, program: &Path) -> Command {
        let mut c = Command::new(program);
        c.env("WINEPREFIX", &self.prefix)
            .env("WINEARCH", "win64")
            .env("WINEDEBUG", "-all")
            // No Mono/Gecko install prompts, no desktop menu entries.
            .env("WINEDLLOVERRIDES", "mscoree=;mshtml=;winemenubuilder.exe=d")
            // Fully headless: nothing here needs a window.
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null());
        c
    }

    fn stage_dir(&self) -> PathBuf {
        self.prefix.join("drive_c").join(STAGE)
    }

    /// Copy bridge + DLL + key into the prefix when missing or changed.
    fn stage(&self) -> Result<(), String> {
        let dir = self.stage_dir();
        paths::ensure_private_dir(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        for f in [BRIDGE_EXE, ROOTGO_DLL, APIKEY] {
            let src = self.resources.join(f);
            let dst = dir.join(f);
            let same = match (fs::metadata(&src), fs::metadata(&dst)) {
                (Ok(a), Ok(b)) => a.len() == b.len() && fs::read(&src).ok() == fs::read(&dst).ok(),
                _ => false,
            };
            if !same {
                let tmp = dst.with_extension("partial");
                fs::copy(&src, &tmp).map_err(|e| format!("copy {}: {e}", src.display()))?;
                fs::rename(&tmp, &dst).map_err(|e| format!("install {}: {e}", dst.display()))?;
            }
        }
        Ok(())
    }

    /// Kill anything left in this prefix (used after a timeout).
    pub fn kill_wineserver(&self) {
        if let Some(ws) = &self.wineserver {
            let _ = self.command(ws).arg("-k").stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }

    /// Run one bridge command. `data` is the persisted `Data` token the
    /// Windows client passes as the first argument of every call.
    pub fn call(&self, cmd: &str, data: &str, param_json: &str, timeout: Duration) -> Result<CallResult, String> {
        paths::ensure_private_dir(&paths::data_dir()).map_err(|e| e.to_string())?;
        self.stage()?;
        let dir = self.stage_dir();
        let tag = format!("{}-{}", std::process::id(), CALL_SEQ.fetch_add(1, Ordering::Relaxed));
        let data_name = format!("data-{tag}.txt");
        let param_name = format!("param-{tag}.json");
        let out_name = format!("out-{tag}.txt");
        let cleanup = || {
            for n in [&data_name, &param_name, &out_name] {
                let _ = fs::remove_file(dir.join(n));
            }
        };
        paths::write_private(&dir.join(&data_name), data.as_bytes()).map_err(|e| e.to_string())?;
        paths::write_private(&dir.join(&param_name), param_json.as_bytes()).map_err(|e| e.to_string())?;

        let win = |n: &str| format!("C:\\{STAGE}\\{n}");
        let mut child = self
            .command(&self.wine)
            .current_dir(&dir)
            .arg(dir.join(BRIDGE_EXE))
            .args([cmd.to_string(), win(APIKEY), win(&data_name), win(&param_name), win(&out_name)])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                cleanup();
                format!("Failed to start Wine ({}): {e}", self.wine.display())
            })?;

        // Drain pipes on threads so a chatty child can never block.
        let mut so = child.stdout.take().unwrap();
        let mut se = child.stderr.take().unwrap();
        let t_out = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = so.read_to_string(&mut s);
            s
        });
        let t_err = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = se.read_to_string(&mut s);
            s
        });

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    self.kill_wineserver();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => {
                    cleanup();
                    return Err(format!("waiting for Wine: {e}"));
                }
            }
        };
        let stdout = t_out.join().unwrap_or_default();
        let stderr = t_err.join().unwrap_or_default();
        let output = fs::read_to_string(dir.join(&out_name)).unwrap_or_default();
        cleanup();

        let Some(status) = status else {
            return Err(format!("The directory service did not answer within {}s", timeout.as_secs()));
        };
        let mut failures = Vec::new();
        let mut log = stdout;
        for line in stderr.lines() {
            if line.trim_start().starts_with('{') {
                failures.push(line.trim().to_string());
            } else {
                log.push_str(line);
                log.push('\n');
            }
        }
        match status.code() {
            Some(0) => Ok(CallResult { output, failures, log }),
            Some(2) => Err(format!("Directory service initialization failed: {}", output.trim())),
            code => Err(format!(
                "rootgo-bridge exited with {}: {}",
                code.map(|c| c.to_string()).unwrap_or_else(|| "a signal".into()),
                log.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
            )),
        }
    }

    /// `VpnService.GetDirectoryServerConfig`: up to `attempts` tries.
    pub fn get_config(&self, data: &str, param: &ConfigParameter, attempts: u32) -> Result<DirectoryServerConfig, String> {
        let param_json = serde_json::to_string(param).map_err(|e| e.to_string())?;
        let mut last_err = String::from("no attempt made");
        for attempt in 0..attempts.max(1) {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(2));
            }
            match self.call("getconfig", data, &param_json, Duration::from_secs(150)) {
                Ok(r) => {
                    let text = r.output.trim();
                    if text.is_empty() || text.eq_ignore_ascii_case("ok") {
                        last_err = "The directory service returned an empty answer".into();
                        continue;
                    }
                    match serde_json::from_str::<DirectoryServerConfig>(text) {
                        Ok(cfg) => return Ok(cfg),
                        Err(_) => last_err = text.chars().take(300).collect(),
                    }
                }
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// `IReportService.Feedback`; `Ok(())` when the library answered "ok"/empty.
    pub fn feedback(&self, data: &str, p: &FeedbackParameter) -> Result<(), String> {
        let json = serde_json::to_string(p).map_err(|e| e.to_string())?;
        let r = self.call("feedback", data, &json, Duration::from_secs(90))?;
        let t = r.output.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("ok") {
            Ok(())
        } else {
            Err(t.to_string())
        }
    }
}
