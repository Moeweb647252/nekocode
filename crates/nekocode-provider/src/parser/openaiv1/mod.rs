use std::collections::HashMap;

use anyhow::anyhow;
use futures_util::StreamExt;

pub mod types;

use crate::sse::ServerSentEvents;
use nekocode_core::provider::{ProviderError, ProviderEvent};
use nekocode_types::generate::Usage;
use nekocode_types::{generate::StopReason, tool::ToolCall};
use types::{
    ChatCompletionStreamResponse, ChatCompletionStreamUsage, FinishReason,
    ChatCompletionStreamDeltaToolCall,
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
    /// Whether we have already emitted the terminal `MessageEnd` event for this
    /// stream. The OpenAI stream may end via `finish_reason: "stop"` on a
    /// chunk, or via `[DONE]` with no terminal chunk, so we track this to make
    /// sure exactly one `MessageEnd` is produced.
    terminated: bool,
    /// Buffered events when multiple tool calls flush at once. We drain these
    /// before reading more from the underlying stream so the agent can emit
    /// all parallel tool calls.
    pending_events: Vec<ProviderEvent>,
}

impl OpenAIV1Stream {
    pub fn new(stream: ServerSentEvents) -> Self {
        Self {
            stream,
            pending_tool_calls: HashMap::new(),
            usage: None,
            terminated: false,
            pending_events: Vec::new(),
        }
    }

    /// Consume the accumulated usage stats from the stream, if any.
    pub fn take_usage(&mut self) -> Option<Usage> {
        self.usage.take().map(|u| Usage {
            total_input: u.prompt_tokens,
            total_output: u.completion_tokens,
            cache_hit: false,
            cache_miss: 0,
        })
    }

    pub async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderError> {
        // First drain any events we buffered (e.g. multiple flushed tool calls).
        if let Some(event) = self.pending_events.pop() {
            return Ok(Some(event));
        }
        while let Some(event) = self.stream.next().await {
            let event = event.map_err(|e| anyhow!("Error reading event: {}", e))?;
            let data = event.data.trim();
            if data == "[DONE]" {
                // Flush any tool calls that never received an explicit
                // finish_reason, then ensure a terminal MessageEnd so the
                // agent's generate loop always terminates.
                self.flush_all_pending_tool_calls();
                return Ok(self.take_termination());
            }
            let chunk: ChatCompletionStreamResponse =
                serde_json::from_str(data).map_err(ProviderError::DeserializationError)?;
            if let Some(event) = self.handle_chunk(&chunk) {
                return Ok(Some(event));
            }
        }
        // Stream ended without `[DONE]`. Flush leftover tool calls and emit a
        // terminal event if we haven't already.
        self.flush_all_pending_tool_calls();
        Ok(self.take_termination())
    }

    fn handle_chunk(&mut self, chunk: &ChatCompletionStreamResponse) -> Option<ProviderEvent> {
        // Capture usage from the final chunk.
        if chunk.usage.is_some() {
            self.usage = chunk.usage.clone();
        }

        for choice in &chunk.choices {
            let delta = &choice.delta;

            if let Some(content) = &delta.content
                && !content.is_empty()
            {
                return Some(ProviderEvent::Content(content.clone()));
            }

            if let Some(reasoning) = &delta.reasoning_content
                && !reasoning.is_empty()
            {
                return Some(ProviderEvent::ReasoningContent(reasoning.clone()));
            }

            if let Some(tool_calls) = &delta.tool_calls {
                for tc in tool_calls {
                    self.apply_tool_call_delta(tc);
                }
            }

            // Flush pending tool calls when the stream signals completion.
            if let Some(finish_reason) = &choice.finish_reason {
                match finish_reason {
                    FinishReason::ToolCalls | FinishReason::FunctionCall => {
                        // Flush every pending tool call, not just one.
                        self.flush_all_pending_tool_calls();
                        if !self.pending_events.is_empty() {
                            return self.pending_events.pop();
                        }
                    }
                    FinishReason::Length => {
                        return self.emit_termination(StopReason::Length);
                    }
                    FinishReason::Stop => {
                        return self.emit_termination(StopReason::Stop);
                    }
                    FinishReason::ContentFilter => {
                        return self.emit_termination(StopReason::Error(
                            "Content filtered".to_string(),
                        ));
                    }
                }
            }
        }
        None
    }

    fn apply_tool_call_delta(&mut self, tc: &ChatCompletionStreamDeltaToolCall) {
        let pending =
            self.pending_tool_calls
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

    /// Drain every pending tool call into `pending_events`, emitted in index
    /// order so parallel tool calls are not lost.
    fn flush_all_pending_tool_calls(&mut self) {
        if self.pending_tool_calls.is_empty() {
            return;
        }
        let mut indices: Vec<usize> = self.pending_tool_calls.keys().copied().collect();
        indices.sort_unstable();
        // pending_events is used as a stack (pop), so push in reverse order to
        // preserve ascending index emission.
        for index in indices.into_iter().rev() {
            if let Some(pending) = self.pending_tool_calls.remove(&index) {
                let args: serde_json::Value =
                    serde_json::from_str(&pending.arguments).unwrap_or(serde_json::Value::Null);
                self.pending_events.push(ProviderEvent::ToolCall(ToolCall {
                    id: pending.id.unwrap_or_default(),
                    name: pending.name.unwrap_or_default(),
                    args,
                }));
            }
        }
    }

    /// Emit a terminal `MessageEnd`, ensuring it is only emitted once.
    fn emit_termination(&mut self, reason: StopReason) -> Option<ProviderEvent> {
        if self.terminated {
            return None;
        }
        self.terminated = true;
        Some(ProviderEvent::MessageEnd(reason))
    }

    /// Return the terminal event if not already emitted (defaults to `Stop`).
    fn take_termination(&mut self) -> Option<ProviderEvent> {
        self.emit_termination(StopReason::Stop)
    }
}
