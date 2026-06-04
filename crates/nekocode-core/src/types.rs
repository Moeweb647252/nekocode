use nekocode_types::generate::{AssistantMessage, Message, StreamEvent};

use crate::provider::ProviderResponse;

#[derive(Debug, Clone, Default)]
pub struct GenerateRequest {
    pub messages: Vec<Message>,
    pub system_prompt: Option<String>,
}

pub struct GenerateResponse {
    message: AssistantMessage,
}

impl GenerateResponse {
    pub fn new() -> Self {
        Self {
            message: AssistantMessage { blocks: Vec::new() },
        }
    }

    pub fn merge(&mut self, response: ProviderResponse) {
        todo!()
    }

    pub fn merge_stream_event(&mut self, event: StreamEvent) {
        todo!()
    }
}

impl From<ProviderResponse> for GenerateResponse {
    fn from(_value: ProviderResponse) -> Self {
        todo!()
    }
}
