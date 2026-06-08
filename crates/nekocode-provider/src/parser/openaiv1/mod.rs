use std::collections::HashMap;

use anyhow::anyhow;
use futures_util::StreamExt;

pub mod types;

use crate::sse::ServerSentEvents;
use nekocode_core::provider::{ProviderError, ProviderEvent, ProviderUsage};
use nekocode_types::tool::ToolCall;
use types::{
    ChatCompletionStreamDeltaToolCall, ChatCompletionStreamResponse,
    ChatCompletionStreamUsage, FinishReason,
};

struct PendingOpenAIToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

pub struct OpenAIV1Stream {
    stream: ServerSentEvents,
    pending_tool_calls: HashMap<usize, PendingOpenAIToolCall>,
    usage: Option<ChatCompletionStreamUsage>,
}

impl OpenAIV1Stream {
    pub fn new(stream: ServerSentEvents) -> Self {
        Self {
            stream,
            pending_tool_calls: HashMap::new(),
            usage: None,
        }
    }

    /// Consume the accumulated usage stats from the stream, if any.
    pub fn take_usage(&mut self) -> Option<ProviderUsage> {
        self.usage.take().map(|u| ProviderUsage {
            total_input: u.prompt_tokens,
            total_output: u.completion_tokens,
            cache_hit: false,
            cache_miss: 0,
        })
    }

    pub async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderError> {
        while let Some(event) = self.stream.next().await {
            let event = event.map_err(|e| anyhow!("Error reading event: {}", e))?;
            let data = event.data.trim();
            if data == "[DONE]" {
                return Ok(self.flush_pending_tool_calls().or(Some(ProviderEvent::MessageEnd)));
            }
            let chunk: ChatCompletionStreamResponse = serde_json::from_str(data)
                .map_err(|e| ProviderError::DeserializationError(e))?;
            if let Some(event) = self.handle_chunk(&chunk) {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    fn handle_chunk(&mut self, chunk: &ChatCompletionStreamResponse) -> Option<ProviderEvent> {
        // Capture usage from the final chunk.
        if chunk.usage.is_some() {
            self.usage = chunk.usage.clone();
        }

        for choice in &chunk.choices {
            let delta = &choice.delta;

            if let Some(content) = &delta.content {
                if !content.is_empty() {
                    return Some(ProviderEvent::Content(content.clone()));
                }
            }

            if let Some(reasoning) = &delta.reasoning_content {
                if !reasoning.is_empty() {
                    return Some(ProviderEvent::ReasoningContent(reasoning.clone()));
                }
            }

            if let Some(tool_calls) = &delta.tool_calls {
                for tc in tool_calls {
                    self.apply_tool_call_delta(tc);
                }
            }

            // Flush pending tool calls when the stream signals completion.
            if let Some(finish_reason) = &choice.finish_reason {
                match finish_reason {
                    FinishReason::ToolCalls | FinishReason::Stop => {
                        if let Some(event) = self.flush_pending_tool_calls() {
                            return Some(event);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn apply_tool_call_delta(&mut self, tc: &ChatCompletionStreamDeltaToolCall) {
        let pending = self
            .pending_tool_calls
            .entry(tc.index)
            .or_insert_with(|| PendingOpenAIToolCall {
                id: None,
                name: None,
                arguments: String::new(),
            });
        if let Some(id) = &tc.id {
            pending.id = Some(id.clone());
        }
        if let Some(func) = &tc.function {
            if let Some(name) = &func.name {
                pending.name = Some(name.clone());
            }
            if let Some(args) = &func.arguments {
                pending.arguments.push_str(args);
            }
        }
    }

    fn flush_pending_tool_calls(&mut self) -> Option<ProviderEvent> {
        let indices: Vec<usize> = self.pending_tool_calls.keys().copied().collect();
        for index in indices {
            if let Some(pending) = self.pending_tool_calls.remove(&index) {
                let args: serde_json::Value = serde_json::from_str(&pending.arguments)
                    .unwrap_or(serde_json::Value::Null);
                return Some(ProviderEvent::ToolCall(ToolCall {
                    id: pending.id.unwrap_or_default(),
                    name: pending.name.unwrap_or_default(),
                    args,
                }));
            }
        }
        None
    }
}
