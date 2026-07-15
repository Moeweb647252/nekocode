use nekocode_types::generate::{MessageType, StreamEvent, StreamEventData};

use crate::provider::ProviderResponse;

/// A single provider generation request: the conversation so far, an optional
/// system prompt, and the tool specs the model may call. Built by the agent
/// run loop from the thread's persisted messages + the registry.
#[derive(Debug, Clone, Default)]
pub struct GenerateRequest {
    pub messages: Vec<MessageType>,
    pub system_prompt: Option<String>,
    pub(crate) tool_specs: Vec<nekocode_types::tool::ToolSpec>,
}

impl GenerateRequest {
    pub fn tool_specs(&self) -> &[nekocode_types::tool::ToolSpec] {
        &self.tool_specs
    }
}

/// The accumulated assistant messages from a run, merged from provider
/// responses and tool-call-result stream events. Used by middleware in
/// `after_generate` to inspect what was produced.
#[derive(Debug, Clone, Default)]
pub struct GenerateResponse {
    pub message: Vec<MessageType>,
}

impl GenerateResponse {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge a completed provider response by converting its assistant
    /// message into a [`MessageType`] and appending it.
    pub fn merge(&mut self, response: ProviderResponse) {
        self.message.push(MessageType::Assistant(response.message));
    }

    /// Handle an incremental stream event. Tool-call results are appended
    /// directly to the message list so middleware can inspect them.
    pub fn merge_stream_event(&mut self, event: StreamEvent) {
        if let StreamEventData::ToolCallResult(result) = event.data {
            self.message.push(MessageType::ToolCallResult(result));
        }
    }
}

impl From<ProviderResponse> for GenerateResponse {
    fn from(value: ProviderResponse) -> Self {
        Self {
            message: vec![MessageType::Assistant(value.message)],
        }
    }
}
