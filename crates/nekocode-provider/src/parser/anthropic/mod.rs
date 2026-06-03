use std::collections::HashMap;

use anyhow::anyhow;
use futures_util::StreamExt;
use nekocode_types::tool::ToolCall;

pub mod types;

use crate::sse::ServerSentEvents;
use nekocode_core::provider::{ProviderError, ProviderEvent};
use types::{ContentBlock, RawContentBlockDelta, RawMessageStreamEvent};

struct PendingToolCall {
    id: String,
    name: String,
    json_fragment: String,
}

pub struct AnthropicStream {
    stream: ServerSentEvents,
    pending_tool_calls: HashMap<usize, PendingToolCall>,
}

impl AnthropicStream {
    pub fn new(stream: ServerSentEvents) -> Self {
        Self {
            stream,
            pending_tool_calls: HashMap::new(),
        }
    }

    pub async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderError> {
        while let Some(event) = self.stream.next().await {
            let event = event.map_err(|e| anyhow!("Error reading event: {}", e))?;
            match event.event_type.as_str() {
                "ping" => continue,
                _ => {
                    let delta: RawMessageStreamEvent = serde_json::from_str(&event.data)
                        .map_err(|e| ProviderError::DeserializationError(e))?;
                    match self.handle_delta(delta) {
                        Some(event) => return Ok(Some(event)),
                        None => continue,
                    }
                }
            }
        }
        Ok(None)
    }

    fn handle_delta(&mut self, delta: RawMessageStreamEvent) -> Option<ProviderEvent> {
        match delta {
            RawMessageStreamEvent::RawMessageStartEvent(start) => {
                for block in &start.message.content {
                    if let Some(event) = self.handle_content_block_start(0, block) {
                        // message_start content blocks don't have indices, assign 0
                        // In practice, there won't be multiple blocks with index 0 in message_start
                        return Some(event);
                    }
                }
                Some(ProviderEvent::MessageStart)
            }

            RawMessageStreamEvent::RawContentBlockStartEvent(start) => {
                if let Some(event) = self.handle_content_block_start(start.index, &start.block) {
                    return Some(event);
                }
                None
            }

            RawMessageStreamEvent::RawContentBlockDeltaEvent(delta) => match delta.delta {
                RawContentBlockDelta::TextDelta(td) => Some(ProviderEvent::Content(td.text)),
                RawContentBlockDelta::InputJsonDelta(ijd) => {
                    if let Some(pending) = self.pending_tool_calls.get_mut(&delta.index) {
                        pending.json_fragment.push_str(&ijd.partial_json);
                    }
                    None
                }
                RawContentBlockDelta::ThinkingDelta(td) => {
                    Some(ProviderEvent::ReasoningContent(td.thinking))
                }
                RawContentBlockDelta::SignatureDelta(sd) => {
                    Some(ProviderEvent::ReasoningContent(sd.signature))
                }
            },

            RawMessageStreamEvent::RawContentBlockStopEvent(stop) => {
                if let Some(pending) = self.pending_tool_calls.remove(&stop.index) {
                    let args: serde_json::Value = serde_json::from_str(&pending.json_fragment)
                        .unwrap_or(serde_json::Value::Null);
                    return Some(ProviderEvent::ToolCall(ToolCall {
                        id: pending.id,
                        name: pending.name,
                        args,
                    }));
                }
                None
            }

            RawMessageStreamEvent::RawMessageDeltaEvent(_) => None,
            RawMessageStreamEvent::RawMessageStopEvent => Some(ProviderEvent::MessageEnd),
        }
    }

    fn handle_content_block_start(
        &mut self,
        index: usize,
        block: &ContentBlock,
    ) -> Option<ProviderEvent> {
        match block {
            ContentBlock::TextBlock { text } if !text.is_empty() => {
                Some(ProviderEvent::Content(text.clone()))
            }
            ContentBlock::ThinkingBlock { thinking, .. } if !thinking.is_empty() => {
                Some(ProviderEvent::ReasoningContent(thinking.clone()))
            }
            ContentBlock::ToolUseBlock { id, name, input } => {
                let input_str = input.to_string();
                if input_str == "{}" {
                    self.pending_tool_calls.insert(
                        index,
                        PendingToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            json_fragment: String::new(),
                        },
                    );
                    None
                } else {
                    Some(ProviderEvent::ToolCall(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        args: input.clone(),
                    }))
                }
            }
            _ => None,
        }
    }
}
