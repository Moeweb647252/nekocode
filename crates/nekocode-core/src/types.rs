use nekocode_types::generate::{Message, StreamEvent, StreamEventData};

use crate::provider::ProviderResponse;

#[derive(Debug, Clone, Default)]
pub struct GenerateRequest {
    pub messages: Vec<Message>,
    pub system_prompt: Option<String>,
}

pub struct GenerateResponse {
    pub message: Vec<Message>,
}

impl GenerateResponse {
    pub fn new() -> Self {
        Self {
            message: Vec::new(),
        }
    }

    /// Merge a completed provider response by converting its assistant
    /// message into a [`Message`] and appending it.
    pub fn merge(&mut self, response: ProviderResponse) {
        self.message.push(Message::Assistant(response.message));
    }

    /// Handle an incremental stream event. Tool-call results are appended
    /// directly to the message list so middleware can inspect them.
    pub fn merge_stream_event(&mut self, event: StreamEvent) {
        match event.data {
            StreamEventData::ToolCallResult(result) => {
                self.message.push(Message::ToolCallResult(result));
            }
            _ => {}
        }
    }
}

impl From<ProviderResponse> for GenerateResponse {
    fn from(value: ProviderResponse) -> Self {
        Self {
            message: vec![Message::Assistant(value.message)],
        }
    }
}
