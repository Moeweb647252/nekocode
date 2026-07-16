use std::collections::HashMap;

use anyhow::anyhow;
use futures_util::StreamExt;
use nekocode_types::{generate::StopReason, tool::ToolCall};

pub(crate) mod types;

use crate::sse::ServerSentEvents;
use nekocode_core::provider::{ProviderError, ProviderEvent};
use nekocode_types::generate::Usage as CoreUsage;
use types::{ContentBlock, RawContentBlockDelta, RawMessageStreamEvent, Usage};

struct PendingToolCall {
    id: String,
    name: String,
    json_fragment: String,
}

pub(crate) struct AnthropicStream {
    stream: ServerSentEvents,
    pending_tool_calls: HashMap<usize, PendingToolCall>,
    usage: Option<Usage>,
    /// Tracks the final stop reason reported via `message_delta`. Defaults to
    /// `Stop` (end_turn) when the provider omits it.
    stop_reason: Option<types::StopReason>,
}

impl AnthropicStream {
    pub(crate) fn new(stream: ServerSentEvents) -> Self {
        Self {
            stream,
            pending_tool_calls: HashMap::new(),
            usage: None,
            stop_reason: None,
        }
    }

    /// Consume the accumulated usage stats from the stream, if any.
    pub(crate) fn take_usage(&mut self) -> Option<CoreUsage> {
        self.usage.take().map(|u| CoreUsage {
            total_input: u.input_tokens,
            total_output: u.output_tokens,
            cache_hit: u.cache_read_input_tokens > 0,
            cache_miss: u.cache_creation_input_tokens,
        })
    }

    pub(crate) async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderError> {
        while let Some(event) = self.stream.next().await {
            let event = event.map_err(|e| anyhow!("Error reading event: {}", e))?;
            match event.event_type.as_str() {
                "ping" => continue,
                _ => {
                    let delta: RawMessageStreamEvent = serde_json::from_str(&event.data)
                        .map_err(ProviderError::DeserializationError)?;
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
                self.usage = Some(start.message.usage.clone());
                for (i, block) in start.message.content.iter().enumerate() {
                    if let Some(event) = self.handle_content_block_start(i, block) {
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
                // The signature is an opaque verification token the server
                // requires to be echoed back for multi-turn thinking. We do
                // not have a place to store it yet, so we drop the delta
                // rather than pollute the reasoning content stream (which
                // previously caused invalid signatures to be sent back).
                RawContentBlockDelta::SignatureDelta(_) => None,
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

            // The message_delta carries the authoritative stop_reason and the
            // final (cumulative) output token count. Record both.
            RawMessageStreamEvent::RawMessageDeltaEvent(d) => {
                if let Some(reason) = d.delta.stop_reason {
                    self.stop_reason = Some(reason);
                }
                if let Some(usage) = d.usage
                    && let Some(self_usage) = self.usage.as_mut()
                {
                    self_usage.output_tokens = usage.output_tokens;
                }
                None
            }
            RawMessageStreamEvent::RawMessageStopEvent => {
                Some(ProviderEvent::MessageEnd(self.map_stop_reason()))
            }
        }
    }

    fn map_stop_reason(&self) -> StopReason {
        match self.stop_reason {
            Some(types::StopReason::MaxTokens) => StopReason::Length,
            Some(types::StopReason::Refusal) => {
                StopReason::Error("Model refused the request".to_string())
            }
            // end_turn / stop_sequence / tool_use / pause_turn all map to a
            // normal stop from our perspective.
            _ => StopReason::Stop,
        }
    }

    fn handle_content_block_start(
        &mut self,
        index: usize,
        block: &ContentBlock,
    ) -> Option<ProviderEvent> {
        match block {
            // Anthropic streams text via `text_delta` events; the initial
            // `content_block_start` text is normally empty. Only emit it when
            // non-empty to avoid duplicating content that deltas will resend.
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
