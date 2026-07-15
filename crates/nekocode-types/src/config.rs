use serde::{Deserialize, Serialize};

/// Root of the server's TOML configuration (`~/.config/nekocode/config.toml`).
///
/// Holds the auth scheme, the available model backends plus the name of the
/// default one, the HTTP listen settings, the app-level section, and the
/// skills directory. Deserialized once at startup and shared behind an
/// `Arc<RwLock<Config>>` in the server's `AppState`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub auth: AuthenticationConfig,
    pub models: Vec<ModelConfig>,
    pub default_model: String,
    pub server: ServerConfig,
    pub app: AppConfig,
    #[serde(default)]
    pub skills: SkillsDirectoryConfig,
}

/// Global configuration for the skills middleware — where user-defined
/// skill directories live. Builtin skills are compiled into the binary
/// and need no path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsDirectoryConfig {
    /// Directory containing user-defined skill subdirectories (each
    /// `<name>/SKILL.md`). Per the agentskills.io spec, single-file
    /// skills are not supported — only directories.
    #[serde(default = "default_skills_directory")]
    pub directory: String,
}

impl Default for SkillsDirectoryConfig {
    fn default() -> Self {
        Self {
            directory: default_skills_directory(),
        }
    }
}

fn default_skills_directory() -> String {
    dirs::config_dir()
        .map(|p| p.join("nekocode"))
        .unwrap_or_default()
        .join("skills")
        .display()
        .to_string()
}

/// One entry in `Config.models`: a user-chosen `name` plus the
/// backend-specific settings. The `name` is what `default_model` (and the
/// API) refers to; `data` carries the typed backend config via a flattened
/// [`ModelConfigType`] tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    #[serde(flatten)]
    pub data: ModelConfigType,
}

/// Backend discriminator for a [`ModelConfig`], tagged by `"type"`. Today
/// only the DeepSeek-compatible backend exists; it dispatches to either the
/// Anthropic or OpenAI wire format via [`DeepSeekEndpoint`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ModelConfigType {
    #[serde(rename = "deepseek")]
    DeepSeek(DeepSeekConfig),
}

/// Connection and sampling parameters for the DeepSeek-compatible backend.
/// Despite the name, the same config drives the Anthropic-format endpoint
/// when [`endpoint`](DeepSeekConfig::endpoint) is set to
/// [`Anthropic`](DeepSeekEndpoint::Anthropic).
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

/// Which wire format and endpoint shape to use against the configured
/// `api_base`. `OpenAI` (the default) speaks the `/v1/chat/completions`
/// protocol; `Anthropic` speaks the `/v1/messages` protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeepSeekEndpoint {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai")]
    OpenAI,
}

/// Authentication scheme for the HTTP API. `Password` requires clients to
/// send a matching `Token` header (validated against this password, with a
/// 30-day UUID token minted on login); `None` leaves every `/api` route
/// open. Defaults to [`None`](AuthenticationConfig::None).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthenticationConfig {
    #[serde(rename = "password")]
    Password { password: String },
    #[serde(rename = "none")]
    #[default]
    None,
}

/// HTTP listen settings. Defaults to `localhost:51211`.
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

/// Placeholder for app-level configuration ([`Config::app`]). Currently empty;
/// kept as a typed section so future app settings can land without touching
/// the rest of the schema.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {}
