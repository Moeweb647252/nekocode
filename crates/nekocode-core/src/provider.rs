use nekocode_types::{
    generate::{self, AssistantMessage},
    tool::ToolCall,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::types::GenerateRequest;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("Error while deserializing data: {0}")]
    DeserializationError(serde_json::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn stream_generate(
        &self,
        request: GenerateRequest,
        sender: UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError>;

    async fn generate(&self, request: GenerateRequest) -> Result<ProviderResponse, ProviderError>;
}

#[derive(Debug, Clone)]
pub enum Role {
    User,
    Assistant,
    Tool,
    Custom(String),
}

#[derive(Debug, Clone)]
pub enum MessageContent {
    Text(String),
}

pub struct ProviderResponse {
    pub message: AssistantMessage,
    pub usage: ProviderUsage,
}

pub struct ProviderUsage {
    pub total_input: usize,
    pub total_output: usize,
    pub cache_hit: bool,
    pub cache_miss: usize,
}

pub enum ProviderEvent {
    MessageStart,
    MessageEnd,
    Content(String),
    ReasoningContent(String),
    ToolCall(ToolCall),
}

impl Into<generate::StreamEvent> for &ProviderEvent {
    fn into(self) -> generate::StreamEvent {
        todo!()
    }
}
