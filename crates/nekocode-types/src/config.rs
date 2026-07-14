use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthenticationConfig {
    #[serde(rename = "password")]
    Password { password: String },
    #[serde(rename = "none")]
    #[default]
    None,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {}
