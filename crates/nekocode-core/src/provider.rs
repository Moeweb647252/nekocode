use nekocode_types::{
    generate::{self, AssistantMessage, StopReason, Usage},
    tool::ToolCall,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::types::GenerateRequest;

/// Errors a [`Provider`] can surface: HTTP/transport failures, deserialization
/// failures, or any backend-specific error wrapped in `Other`.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    HttpError(String),
    #[error("Error while deserializing data: {0}")]
    DeserializationError(#[from] serde_json::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The LLM backend abstraction. A provider streams a single generation's
/// events through `sender` as they arrive (so the client can render live),
/// then returns the finalized [`ProviderResponse`] (assistant message + usage).
/// Implementations live in `nekocode-provider`; the agent only talks to this
/// trait.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn stream_generate(
        &self,
        request: GenerateRequest,
        sender: UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError>;
}

/// The finalized result of one provider generation: the assistant's message
/// and its token usage.
pub struct ProviderResponse {
    pub message: AssistantMessage,
    pub usage: Usage,
}

/// Incremental events yielded by [`Provider::stream_generate`] during a single
/// generation. `MessageStart`/`MessageEnd` bracket the stream; `Content` /
/// `ReasoningContent` carry text deltas; `ToolCall` carries a parsed tool
/// request. Converted to a [`generate::StreamEvent`] by the agent run loop.
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
