use nekocode_types::{
    generate::{self, AssistantMessage, StopReason, Usage},
    tool::ToolCall,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::types::GenerateRequest;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("Error while deserializing data: {0}")]
    DeserializationError(#[from] serde_json::Error),
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
}

pub struct ProviderResponse {
    pub message: AssistantMessage,
    pub usage: Usage,
}

#[derive(Clone)]
pub enum ProviderEvent {
    MessageStart,
    MessageEnd(StopReason),
    Content(String),
    ReasoningContent(String),
    ToolCall(ToolCall),
}

impl From<&ProviderEvent> for generate::StreamEvent {
    fn from(event: &ProviderEvent) -> generate::StreamEvent {
        let data = match event {
            ProviderEvent::MessageStart => {
                generate::StreamEventData::MessageStart(generate::MessageMetadata {
                    role: generate::Role::Assistant,
                })
            }
            ProviderEvent::MessageEnd(reason) => {
                generate::StreamEventData::MessageEnd(reason.clone())
            }
            ProviderEvent::Content(text) => generate::StreamEventData::Content(text.clone()),
            ProviderEvent::ReasoningContent(text) => {
                generate::StreamEventData::ReasoningContent(text.clone())
            }
            ProviderEvent::ToolCall(tc) => generate::StreamEventData::ToolCall(tc.clone()),
        };
        generate::StreamEvent {
            data,
            created_at: jiff::Timestamp::now(),
        }
    }
}
