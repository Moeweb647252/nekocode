use nekocode_types::{
    generate,
    tool::{ToolCall, ToolCallResult},
};
use serde::Serialize;
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
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
    pub reasoning_content: Option<String>,
}

pub fn collect_db_messages(messages: Vec<nekocode_entities::message::Message>) -> Vec<Message> {
    todo!()
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOption {
    pub model: String,
    pub max_tokens: Option<usize>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone)]
pub enum Role {
    User,
    Assistant,
    Custom(String),
}

#[derive(Debug, Clone)]
pub enum MessageContent {
    Text(String),
}

pub struct ProviderResponse {}

pub enum ProviderEvent {
    MessageStart,
    MessageEnd,
    Content(String),
    ReasoningContent(String),
    ToolCall(ProviderToolCall),
    ToolCallResult(ToolCallResult),
}

pub struct ProviderToolCall {}

impl Into<generate::StreamEvent> for &ProviderEvent {
    fn into(self) -> generate::StreamEvent {
        todo!()
    }
}
