//! XDG locations and bundled resources.

use std::env;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(v) if Path::new(&v).is_absolute() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

/// `~/.config/nthlink`: client id, persisted `Data`, settings.
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("nthlink")
}

/// `~/.local/share/nthlink`: the Wine prefix.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("nthlink")
}

/// `~/.cache/nthlink`: generated leaf config, logs, last good directory config.
pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache").join("nthlink")
}

pub fn log_dir() -> PathBuf {
    cache_dir().join("logs")
}

/// Directory holding `rootgo-bridge.exe`, `rootGo.dll`, `apikey` and the
/// `nthlink-core` binary. Overridable with `NTHLINK_RESOURCES`.
pub fn resource_dir() -> Option<PathBuf> {
    if let Some(p) = env::var_os("NTHLINK_RESOURCES") {
        return Some(PathBuf::from(p));
    }
    let exe = env::current_exe().ok()?;
    let bin = exe.parent()?;
    for cand in [bin.join("../lib/nthlink"), bin.to_path_buf(), bin.join("../../resources")] {
        if cand.join("rootGo.dll").is_file() {
            return cand.canonicalize().ok();
        }
    }
    None
}

/// Create a directory (and parents) readable only by the user.
pub fn ensure_private_dir(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(p)?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
}

/// Write a file atomically with mode 0600.
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}
