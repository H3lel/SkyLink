//! Data model of the nthLink directory service, mirroring the field names of
//! `nthLink.Header.Struct` in the Windows client so the JSON returned by
//! rootGo.dll deserializes the same way it does there.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    pub protocol: String,
    pub host: String,
    pub port: String,
    pub password: String,
    pub sni: String,
    pub encrypt_method: String,
    pub ws: bool,
    pub ws_path: String,
    pub ws_host: String,
    pub ips: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HeadlineNews {
    pub title: String,
    pub excerpt: String,
    pub image: String,
    pub url: String,
    #[serde(rename = "pinToTop")]
    pub pin_to_top: bool,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Notification {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Platform {
    pub os: String,
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CurrentVersion {
    pub app_name: String,
    pub platforms: Vec<Platform>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Altern {
    pub app_name: String,
    pub website: String,
    pub app_store_macos: String,
    pub app_store_ios: String,
    pub app_store_android: String,
    pub app_store_windows: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceAvail {
    pub is_avail: bool,
    pub has_altern: bool,
    pub altern: Option<Altern>,
}

/// `DirectoryServerConfig` – the decrypted answer of `GetConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectoryServerConfig {
    #[serde(rename = "clientId")]
    pub client_id: String,
    pub servers: Vec<Server>,
    #[serde(rename = "redirectUrl")]
    pub redirect_url: String,
    #[serde(rename = "headlineNews")]
    pub headline_news: Vec<HeadlineNews>,
    pub notifications: Vec<Notification>,
    pub data: Option<String>,
    pub use_custom_config: bool,
    pub custom_config: String,
    pub current_versions: Vec<CurrentVersion>,
    pub service_avail: Option<ServiceAvail>,
}

impl DirectoryServerConfig {
    /// A config is usable when it carries either a custom leaf config or at
    /// least one server (the same condition the Windows client acts on).
    pub fn is_usable(&self) -> bool {
        (self.use_custom_config && !self.custom_config.trim().is_empty()) || !self.servers.is_empty()
    }

    /// `service_avail.is_avail == false` means the service is closed for this
    /// client; a missing object means available.
    pub fn service_unavailable(&self) -> Option<&ServiceAvail> {
        self.service_avail.as_ref().filter(|s| !s.is_avail)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceParams {
    #[serde(rename = "Os")]
    pub os: String,
    #[serde(rename = "OsVersion")]
    pub os_version: String,
}

/// `ConfigParameter`, serialized with Newtonsoft's default (PascalCase field
/// names) exactly as the Windows SDK does.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigParameter {
    #[serde(rename = "ClientId")]
    pub client_id: String,
    #[serde(rename = "Language")]
    pub language: String,
    #[serde(rename = "Device")]
    pub device: DeviceParams,
    #[serde(rename = "AppVersion")]
    pub app_version: String,
    #[serde(rename = "SdkVersion")]
    pub sdk_version: String,
    #[serde(rename = "Timezone")]
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackParameter {
    #[serde(rename = "ClientId")]
    pub client_id: String,
    #[serde(rename = "Language")]
    pub language: String,
    #[serde(rename = "Os")]
    pub os: String,
    #[serde(rename = "AppVersion")]
    pub app_version: String,
    #[serde(rename = "UtcSent")]
    pub utc_sent: String,
    #[serde(rename = "FeedbackType")]
    pub feedback_type: String,
    #[serde(rename = "Description")]
    pub description: String,
    #[serde(rename = "ErrorCode")]
    pub error_code: Option<String>,
    #[serde(rename = "ErrorMessage")]
    pub error_message: Option<String>,
    #[serde(rename = "Email")]
    pub email: String,
}

/// Version identity reported to the directory service. The API key shipped
/// with the Windows build is bound to that client, so by default we report the
/// same identity (`Windows`, app 7.1.0.0, SDK 0.0.3.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientIdentity {
    pub os: String,
    pub os_version: String,
    pub app_version: String,
    pub sdk_version: String,
}

impl Default for ClientIdentity {
    fn default() -> Self {
        Self {
            os: "Windows".into(),
            os_version: "Microsoft Windows NT 10.0.19045.0".into(),
            app_version: "7.1.0.0".into(),
            sdk_version: "0.0.3.0".into(),
        }
    }
}
