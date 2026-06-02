use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub auth: AuthenticationConfig,
    pub providers: Vec<ProviderConfig>,
    pub server: ServerConfig,
    pub app: AppConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {}

impl Default for ProviderConfig {
    fn default() -> Self {
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthenticationConfig {
    #[serde(rename = "password")]
    Password { password: String },
    #[serde(rename = "none")]
    None,
}

impl Default for AuthenticationConfig {
    fn default() -> Self {
        AuthenticationConfig::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            host: "localhost".to_string(),
            port: 51211,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub db_path: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            db_path: dirs::config_dir()
                .unwrap_or(PathBuf::new())
                .join("data.db")
                .to_string_lossy()
                .into_owned(),
        }
    }
}
