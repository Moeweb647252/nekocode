use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub auth: AuthenticationConfig,
    pub models: Vec<ModelConfig>,
    pub default_model: String,
    pub server: ServerConfig,
    pub app: AppConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    #[serde(flatten)]
    pub data: ModelConfigType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ModelConfigType {
    #[serde(rename = "deepseek")]
    DeepSeek(DeepSeekConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepSeekConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub max_tokens: Option<usize>,
    pub context_window_size: Option<usize>,
    pub endpoint: Option<DeepSeekEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeepSeekEndpoint {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai")]
    OpenAI,
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
