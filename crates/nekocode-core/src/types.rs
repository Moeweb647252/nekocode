use nekocode_types::generate::{MessageType, StreamEvent, StreamEventData};

use crate::provider::ProviderResponse;

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

#[derive(Debug, Clone, Default)]
pub struct GenerateResponse {
    pub message: Vec<MessageType>,
}

impl GenerateResponse {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge a completed provider response by converting its assistant
    /// message into a [`Message`] and appending it.
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
