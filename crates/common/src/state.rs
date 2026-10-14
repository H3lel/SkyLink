//! Persistent client state, the Linux counterpart of `ClientInfoImp` +
//! `DataProtector` (which stores the same keys DPAPI-encrypted on Windows).

use crate::model::{ClientIdentity, ConfigParameter, DeviceParams, DirectoryServerConfig};
use crate::paths;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Open the server's landing page (`redirectUrl`) after connecting, as the
    /// Windows client does.
    pub open_landing_page: bool,
    /// leaf log level.
    pub log_level: String,
    /// Identity reported to the directory service.
    pub identity: ClientIdentity,
}

impl Default for Settings {
    fn default() -> Self {
        Self { open_landing_page: true, log_level: "warn".into(), identity: ClientIdentity::default() }
    }
}

#[derive(Debug, Clone)]
pub struct ClientState {
    pub client_guid: String,
    pub data: String,
    pub settings: Settings,
}

fn new_guid() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn read_trim(name: &str) -> Option<String> {
    fs::read_to_string(paths::config_dir().join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl ClientState {
    pub fn load() -> Self {
        let dir = paths::config_dir();
        let _ = paths::ensure_private_dir(&dir);
        let client_guid = match read_trim("ClientGuid") {
            Some(g) => g,
            None => {
                let g = new_guid();
                let _ = paths::write_private(&dir.join("ClientGuid"), g.as_bytes());
                g
            }
        };
        let data = read_trim("Data").unwrap_or_default();
        let settings = fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { client_guid, data, settings }
    }

    pub fn update_data(&mut self, data: &str) {
        self.data = data.to_string();
        let _ = paths::write_private(&paths::config_dir().join("Data"), data.as_bytes());
    }

    pub fn save_settings(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.settings) {
            let _ = paths::write_private(&paths::config_dir().join("settings.json"), s.as_bytes());
        }
    }

    /// `VpnService.MakeConfigParameter`.
    pub fn config_parameter(&self) -> ConfigParameter {
        let id = &self.settings.identity;
        ConfigParameter {
            client_id: self.client_guid.clone(),
            language: culture_name(),
            device: DeviceParams { os: id.os.clone(), os_version: id.os_version.clone() },
            app_version: id.app_version.clone(),
            sdk_version: id.sdk_version.clone(),
            timezone: timezone_string(),
        }
    }

    pub fn save_last_config(cfg: &DirectoryServerConfig) {
        let dir = paths::cache_dir();
        let _ = paths::ensure_private_dir(&dir);
        if let Ok(s) = serde_json::to_vec(cfg) {
            let _ = paths::write_private(&dir.join("last-config.json"), &s);
        }
    }

    pub fn load_last_config() -> Option<DirectoryServerConfig> {
        let s = fs::read(paths::cache_dir().join("last-config.json")).ok()?;
        serde_json::from_slice(&s).ok()
    }
}

/// .NET `CultureInfo.CurrentCulture.Name` from the POSIX locale
/// (`fa_IR.UTF-8` -> `fa-IR`).
pub fn culture_name() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let base = v.split(['.', '@']).next().unwrap_or("").trim();
            if base.is_empty() || base == "C" || base == "POSIX" {
                continue;
            }
            return base.replace('_', "-");
        }
    }
    "en-US".into()
}

/// The exact string the Windows client builds:
/// `"GMT" + sign + offset.ToString("hh").TrimStart('0')`.
pub fn timezone_string() -> String {
    let offset = local_utc_offset_secs();
    let sign = if offset < 0 { '-' } else { '+' };
    let hours = format!("{:02}", (offset.abs() / 3600) % 24);
    format!("GMT{sign}{}", hours.trim_start_matches('0'))
}

fn local_utc_offset_secs() -> i64 {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff
    }
}

/// `DateTime.UtcNow.ToString("yyyy-MM-dd HH:mm:ss")`.
pub fn utc_now_string() -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&now, &mut tm);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn guid_shape() {
        let g = super::new_guid();
        assert_eq!(g.len(), 36);
        assert_eq!(&g[14..15], "4");
    }
}
