use anyhow::anyhow;
use futures_util::StreamExt;
mod types;

use crate::{parser::anthropic::types::RawMessageStreamEvent, sse::ServerSentEvents};

use nekocode_core::provider::{ProviderError, ProviderEvent};

pub struct AnthropicStream {
    stream: ServerSentEvents,
}

impl AnthropicStream {
    pub fn new(stream: ServerSentEvents) -> Self {
        Self { stream }
    }

    pub async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderError> {
        while let Some(event) = self.stream.next().await {
            let event = event.map_err(|e| anyhow!("Error reading event: {}", e))?;
            match event.event_type.as_str() {
                "ping" => continue,
                _ => {
                    let delta: RawMessageStreamEvent = serde_json::from_str(&event.data)
                        .map_err(|e| ProviderError::DeserializationError(e))?;
                }
            }
        }
        Ok(None)
    }
}
