use nekocode_types::generate::{
    AssistantContentBlock, AssistantMessage, Message, MessageContent, MessageMetadata, Role,
    StreamEvent, StreamEventData, Usage,
};
use nekocode_types::tool::{ToolCallResult, ToolCallResultInner, ToolRegistry};
use tokio::sync::mpsc::UnboundedSender;

use super::error::AgentError;
use crate::{
    agent::AgentEvent,
    agent::AgentEventType,
    middleware::{AgentControlFlow, Middleware},
    provider::ProviderEvent,
    types::{GenerateRequest, GenerateResponse},
};

/// Abstraction over message persistence for the agent run loop.
///
/// The run loop has two implementations: one backed by a database
/// (`DbMessageStore`, used by `Agent`) and one backed by an in-memory
/// `Vec` (`InMemoryMessageStore`, used by `SubAgent`). The trait
/// isolates the persistence concern so the loop logic can be shared.
#[async_trait::async_trait]
pub trait MessageStore: Send + Sync {
    /// Append a user message to the conversation and return the new
    /// message count.
    async fn push_user_message(
        &self,
        content: MessageContent,
    ) -> Result<usize, AgentError>;

    /// Append an assistant message (with usage) and return the new
    /// message count.
    async fn push_assistant_message(
        &self,
        message: AssistantMessage,
        usage: Usage,
    ) -> Result<usize, AgentError>;

    /// Append a tool-call result and return the new message count.
    async fn push_tool_result(
        &self,
        result: ToolCallResult,
    ) -> Result<usize, AgentError>;

    /// Append a middleware-injected message and return the new count.
    async fn push_middleware_message(
        &self,
        content: MessageContent,
    ) -> Result<usize, AgentError>;

    /// Return the current full message history. This is used to build
    /// the `GenerateRequest::messages` field before each provider call.
    async fn current_messages(&self) -> Result<Vec<Message>, AgentError>;

    /// Finalize the run — e.g. mark the turn as finished and persist
    /// accumulated usage. In-memory stores are a no-op.
    async fn finalize(&self, usage: &Usage) -> Result<(), AgentError>;
}

// ──────────────────────────────────────────────────────────────────
// In-memory implementation (used by SubAgent)
// ──────────────────────────────────────────────────────────────────

/// In-memory message store backed by a `RwLock<Vec<Message>>`.
/// Used by `SubAgent` which never touches the database.
pub struct InMemoryMessageStore {
    messages: tokio::sync::RwLock<Vec<Message>>,
}

impl InMemoryMessageStore {
    pub fn new(seed: Vec<Message>) -> Self {
        Self {
            messages: tokio::sync::RwLock::new(seed),
        }
    }

    /// Take the message history out, consuming the store.
    pub async fn into_messages(self) -> Vec<Message> {
        self.messages.into_inner().into_iter().collect()
    }
}

#[async_trait::async_trait]
impl MessageStore for InMemoryMessageStore {
    async fn push_user_message(
        &self,
        content: MessageContent,
    ) -> Result<usize, AgentError> {
        let mut guard = self.messages.write().await;
        guard.push(Message::User(content));
        Ok(guard.len())
    }

    async fn push_assistant_message(
        &self,
        message: AssistantMessage,
        _usage: Usage,
    ) -> Result<usize, AgentError> {
        let mut guard = self.messages.write().await;
        guard.push(Message::Assistant(message));
        Ok(guard.len())
    }

    async fn push_tool_result(
        &self,
        result: ToolCallResult,
    ) -> Result<usize, AgentError> {
        let mut guard = self.messages.write().await;
        guard.push(Message::ToolCallResult(result));
        Ok(guard.len())
    }

    async fn push_middleware_message(
        &self,
        content: MessageContent,
    ) -> Result<usize, AgentError> {
        let mut guard = self.messages.write().await;
        guard.push(Message::MiddlewareMessage(content));
        Ok(guard.len())
    }

    async fn current_messages(&self) -> Result<Vec<Message>, AgentError> {
        Ok(self.messages.read().await.clone())
    }

    async fn finalize(&self, _usage: &Usage) -> Result<(), AgentError> {
        // No-op for in-memory store.
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────
// Shared run-loop core
// ──────────────────────────────────────────────────────────────────

/// Core agent run loop shared by both `Agent` (DB-backed) and `SubAgent`
/// (in-memory). The persistence layer is abstracted through the
/// [`MessageStore`] trait.
///
/// Returns `(index, total_usage)` on success.
pub async fn run_loop_core(
    store: &dyn MessageStore,
    provider: std::sync::Arc<dyn crate::provider::Provider>,
    middlewares: &[Box<dyn Middleware>],
    sender: &Option<UnboundedSender<AgentEvent>>,
    base_system_prompt: Option<String>,
    mut request: GenerateRequest,
    mut index: usize,
) -> Result<(usize, Usage), AgentError> {
    let mut total_usage = Usage::default();

    // ---- Outer middleware loop ---------------------------------------
    loop {
        // 1. before_generate: register tools / mutate request.
        let mut tool_registry = ToolRegistry::new();
        for middleware in middlewares.iter() {
            middleware
                .before_generate(&mut request, &mut tool_registry)
                .await?;
        }
        request.tool_specs = tool_registry.specs();
        let system_prompt = request.system_prompt.clone();
        let tool_specs = request.tool_specs.clone();

        let mut generate_response = GenerateResponse::new();

        // ---- Inner tool-call loop ------------------------------------
        loop {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

            // Emit MessageStart up front, suppressing the provider's
            // duplicate.
            if let Some(s) = sender.as_ref() {
                s.send(AgentEvent {
                    index,
                    data: AgentEventType::StreamEvent(StreamEvent {
                        data: StreamEventData::MessageStart(MessageMetadata {
                            role: Role::Assistant,
                        }),
                        created_at: jiff::Timestamp::now(),
                    }),
                })
                .map_err(|e| AgentError::Other(anyhow::anyhow!("error sending agent event {e}")))?;
                index += 1;
            }

            let request_for_call = request.clone();
            let provider_clone = provider.clone();
            let handle =
                tokio::spawn(async move { provider_clone.stream_generate(request_for_call, tx).await });

            // Forward provider events to the optional sender. The break
            // decision is taken from the *response*, NOT from MessageEnd.
            while let Some(event) = rx.recv().await {
                if matches!(event, ProviderEvent::MessageStart) {
                    continue;
                }
                if let Some(s) = sender.as_ref() {
                    s.send(AgentEvent {
                        index,
                        data: AgentEventType::StreamEvent((&event).into()),
                    })
                    .map_err(|e| {
                        AgentError::Other(anyhow::anyhow!("error sending agent event {e}"))
                    })?;
                    index += 1;
                }
            }

            let response = handle
                .await
                .map_err(|e| -> AgentError { anyhow::anyhow!("error joining task {e}").into() })??;

            // Accumulate usage from this provider call.
            total_usage.total_input += response.usage.total_input;
            total_usage.total_output += response.usage.total_output;
            total_usage.cache_miss += response.usage.cache_miss;
            if response.usage.cache_hit {
                total_usage.cache_hit = true;
            }

            // Persist the assistant message.
            let usage_clone = response.usage.clone();
            store
                .push_assistant_message(response.message.clone(), usage_clone)
                .await?;

            // Execute any tool calls; persist results.
            let mut this_generation_had_tool_calls = false;
            for block in response.message.blocks.iter() {
                if let AssistantContentBlock::ToolCall(tool_call) = block {
                    this_generation_had_tool_calls = true;
                    let tool_call_result = match tool_registry.get(&tool_call.name) {
                        Some(tool) => ToolCallResult {
                            id: tool_call.id.clone(),
                            result: ToolCallResultInner::from(
                                tool.call(tool_call.args.clone()).await,
                            ),
                        },
                        None => ToolCallResult {
                            id: tool_call.id.clone(),
                            result: ToolCallResultInner::Error {
                                error: "Tool not found".into(),
                            },
                        },
                    };
                    store.push_tool_result(tool_call_result.clone()).await?;
                    let stream_event = StreamEvent {
                        data: StreamEventData::ToolCallResult(tool_call_result),
                        created_at: jiff::Timestamp::now(),
                    };
                    if let Some(s) = sender.as_ref() {
                        s.send(AgentEvent {
                            index,
                            data: AgentEventType::StreamEvent(stream_event.clone()),
                        })
                        .map_err(|e| {
                            AgentError::Other(anyhow::anyhow!("error sending agent event {e}"))
                        })?;
                        index += 1;
                    }
                    generate_response.merge_stream_event(stream_event);
                }
            }
            generate_response.merge(response);

            // Inner-loop break decision: stop only when the response had
            // no tool calls. Otherwise feed the just-persisted tool
            // results back into a fresh generation.
            if !this_generation_had_tool_calls {
                break;
            }
            let messages = store.current_messages().await?;
            request = GenerateRequest {
                messages,
                system_prompt: system_prompt.clone(),
                tool_specs: tool_specs.clone(),
            };
        }

        // 2. after_generate hooks.
        let mut control_flow = AgentControlFlow::Output;
        for middleware in middlewares.iter() {
            middleware
                .after_generate(&generate_response, &mut control_flow)
                .await?;
        }
        match control_flow {
            AgentControlFlow::Output => break,
            AgentControlFlow::GenerateWith(content) => {
                store.push_middleware_message(content).await?;
                // Preserve the captured system prompt across the outer
                // middleware-driven regeneration.
                let messages = store.current_messages().await?;
                request = GenerateRequest {
                    messages,
                    system_prompt: base_system_prompt.clone(),
                    ..Default::default()
                };
            }
        }
    }

    // Emit the final TurnEnd event.
    if let Some(s) = sender.as_ref() {
        s.send(AgentEvent {
            index,
            data: AgentEventType::StreamEvent(StreamEvent {
                data: StreamEventData::TurnEnd,
                created_at: jiff::Timestamp::now(),
            }),
        })
        .map_err(|e| AgentError::Other(anyhow::anyhow!("error sending agent event {e}")))?;
    }

    Ok((index, total_usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_mocks::{text_msg, toolcall_msg, EchoMiddleware, MockProvider, OneShotRegenerateMiddleware};
    use crate::middleware::{AgentControlFlow, Middleware};
    use crate::types::GenerateRequest;
    use nekocode_types::generate::MessageContent;
    use nekocode_types::tool::ToolRegistry;
    use std::sync::Arc;

    #[tokio::test]
    async fn text_only_response() {
        let store = InMemoryMessageStore::new(vec![]);
        store
            .push_user_message(MessageContent::Text { content: "hello".into() })
            .await
            .unwrap();

        let provider = Arc::new(MockProvider::new(vec![text_msg("world")]));
        let request = GenerateRequest {
            messages: store.current_messages().await.unwrap(),
            ..Default::default()
        };

        let (_, usage) = run_loop_core(&store, provider, &[], &None, None, request, 0).await.unwrap();

        assert!(usage.total_input > 0);
        let messages = store.current_messages().await.unwrap();
        assert_eq!(messages.len(), 2); // user + assistant
    }

    #[tokio::test]
    async fn tool_call_loop() {
        let store = InMemoryMessageStore::new(vec![]);
        store
            .push_user_message(MessageContent::Text { content: "run a tool".into() })
            .await
            .unwrap();

        let provider = Arc::new(MockProvider::new(vec![
            toolcall_msg("c1", "echo", serde_json::json!({"value": "ping"})),
            text_msg("done"),
        ]));

        let request = GenerateRequest {
            messages: store.current_messages().await.unwrap(),
            ..Default::default()
        };

        let result = run_loop_core(
            &store,
            provider,
            &[Box::new(EchoMiddleware)],
            &None,
            None,
            request,
            0,
        )
        .await;

        assert!(result.is_ok(), "run_loop_core failed: {:?}", result.err());
        let messages = store.current_messages().await.unwrap();
        // user + assistant(toolcall) + toolresult + assistant(text)
        assert_eq!(messages.len(), 4);
    }

    #[tokio::test]
    async fn middleware_generate_with() {
        let store = InMemoryMessageStore::new(vec![]);
        store
            .push_user_message(MessageContent::Text { content: "hello".into() })
            .await
            .unwrap();

        let provider = Arc::new(MockProvider::new(vec![
            text_msg("first"),
            text_msg("second"),
        ]));

        // OneShotRegenerateMiddleware fires GenerateWith on the first
        // after_generate, then leaves flow as Output (its default) on the
        // second call — this exercises the outer loop exactly once.
        let middlewares: Vec<Box<dyn Middleware>> = vec![Box::new(
            OneShotRegenerateMiddleware {
                fired: std::sync::Mutex::new(false),
                inject: "injected".into(),
            },
        )];

        let request = GenerateRequest {
            messages: store.current_messages().await.unwrap(),
            ..Default::default()
        };

        let result = run_loop_core(&store, provider, &middlewares, &None, None, request, 0).await;
        assert!(result.is_ok(), "run_loop_core failed: {:?}", result.err());
        let messages = store.current_messages().await.unwrap();
        // user + first assistant + injected middleware message + second assistant
        assert_eq!(messages.len(), 4);
    }
}

