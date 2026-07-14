//! Decoupled agent run loop.
//!
//! `run_loop` performs generation orchestration only — no DB access. It takes
//! the working history (`old_turns`) and user input, accumulates every message
//! produced during the run in memory, and returns a [`Turn`] carrying the full
//! message list, aggregate usage, and a `finished` flag. The API layer is
//! responsible for persisting the returned turn.
//!
//! On error the run returns `Err(partial_turn)` with `finished = false` and the
//! messages accumulated up to the failure point; the error itself is emitted to
//! the client as a `MessageEnd(StopReason::Error(..))` stream event before the
//! partial turn is returned, so callers don't need to inspect the error out of
//! band.

use nekocode_types::generate::{
    Message, MessageContent, MessageType, StopReason, StreamEvent, StreamEventData, Turn, Usage,
};
use nekocode_types::tool::{ToolCallResult, ToolCallResultInner, ToolRegistry};

use crate::agent::error::AgentError;
use crate::agent::{Agent, AgentEventType};
use crate::middleware::AgentControlFlow;
use crate::provider::ProviderEvent;
use crate::types::{GenerateRequest, GenerateResponse};

impl Agent {
    pub async fn run_loop(
        &self,
        input: Vec<MessageContent>,
        old_turns: Vec<Turn>,
        sink: crate::agent::AgentEventSink,
    ) -> Result<Turn, Turn> {
        // Flatten the working history into a single message list. Owned once,
        // reused as the immutable prefix when rebuilding each generation's
        // request from memory (replaces the DB reload the old loop did).
        let old_messages: Vec<Message> = old_turns
            .into_iter()
            .flat_map(|t| t.messages)
            .collect();

        // The current turn's messages, grown in memory as the run progresses.
        // The user message is the first entry. On error these hold whatever was
        // produced up to the failure point (the partial turn payload).
        let mut current_messages: Vec<Message> = Vec::with_capacity(old_messages.len() + 8);
        current_messages.push(Message {
            created_at: jiff::Timestamp::now(),
            data: MessageType::User(input),
            usage: None,
        });

        // Aggregate token usage across every provider generation in this run.
        let mut total_usage = Usage::default();

        // The system prompt is built once from the agent's working directory
        // (no DB query) and survives both the inner tool-call loop and the
        // outer middleware-driven regeneration loop.
        let base_system_prompt = format!("Working directory: {}\n", self.working_directory);

        // Channel for middlewares to enqueue MiddlewareEvents; a merge relay
        // wraps each into a uniquely-indexed AgentEvent on the parent stream.
        // The relay shares the sink (same atomic index) so its events interleave
        // contiguously with the run's own StreamEvents.
        let (mev_tx, mev_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::agent::MiddlewareEvent>();
        let relay_sink = sink.clone();
        let merge_relay = tokio::spawn(async move {
            let mut mev_rx = mev_rx;
            while let Some(mev) = mev_rx.recv().await {
                // send failure (client gone) just stops relaying
                let _ = relay_sink.send(crate::agent::AgentEventType::MiddlewareEvent(mev));
            }
        });

        // Run the body in an async block that borrows the mutable accumulator
        // state by reference and returns `Result<Turn, AgentError>`. The outer
        // match turns an early `?` return into a partial-turn `Err(Turn)` while
        // preserving everything accumulated so far.
        let result: Result<Turn, AgentError> = async {
            let mut request = GenerateRequest {
                messages: old_messages
                    .iter()
                    .chain(current_messages.iter())
                    .map(|m| m.data.clone())
                    .collect(),
                system_prompt: Some(base_system_prompt.clone()),
                ..Default::default()
            };
            loop {
                let mut tool_registry = ToolRegistry::new();
                for middleware in self.middlewares.iter() {
                    middleware
                        .before_generate(&mut request, &mut tool_registry, &mev_tx)
                        .await?;
                }
                request.tool_specs = tool_registry.specs();
                let system_prompt = request.system_prompt.clone();
                let tool_specs = request.tool_specs.clone();
                let mut generate_response = GenerateResponse::new();
                loop {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                    let provider = self.provider.clone();
                    Self::send(
                        &sink,
                        StreamEventData::MessageStart(nekocode_types::generate::MessageMetadata {
                            role: nekocode_types::generate::Role::Assistant,
                        }),
                    )?;
                    let handle =
                        tokio::spawn(async move { provider.stream_generate(request, tx).await });
                    // Forward provider stream events to the client, skipping
                    // the provider's MessageStart (the agent emits its own
                    // above) so the client doesn't see two per generation.
                    while let Some(event) = rx.recv().await {
                        if matches!(event, ProviderEvent::MessageStart) {
                            continue;
                        }
                        let stream_event: StreamEvent = (&event).into();
                        Self::send(&sink, stream_event.data)?;
                    }
                    let response = handle
                        .await
                        .map_err(|e| AgentError::Other(anyhow::anyhow!("error joining task {e}")))??;
                    // Accumulate usage from this provider call.
                    total_usage.total_input += response.usage.total_input;
                    total_usage.total_output += response.usage.total_output;
                    total_usage.cache_miss += response.usage.cache_miss;
                    if response.usage.cache_hit {
                        total_usage.cache_hit = true;
                    }
                    current_messages.push(Message {
                        created_at: jiff::Timestamp::now(),
                        data: MessageType::Assistant(response.message.clone()),
                        usage: Some(response.usage.clone()),
                    });
                    let mut this_generation_had_tool_calls = false;
                    for block in response.message.blocks.iter() {
                        if let nekocode_types::generate::AssistantContentBlock::ToolCall(tool_call) = block {
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
                            current_messages.push(Message {
                                created_at: jiff::Timestamp::now(),
                                data: MessageType::ToolCallResult(tool_call_result.clone()),
                                usage: None,
                            });
                            let stream_event = StreamEvent {
                                data: StreamEventData::ToolCallResult(tool_call_result),
                                created_at: jiff::Timestamp::now(),
                            };
                            Self::send(
                                &sink,
                                stream_event.data.clone(),
                            )?;
                            generate_response.merge_stream_event(stream_event);
                        }
                    }
                    generate_response.merge(response);
                    // Only break the inner (tool-call) loop when this generation
                    // finished naturally. If it emitted tool calls we've
                    // executed them and appended the results to
                    // `current_messages`, so loop again to feed those results
                    // back into a fresh generation. The decision is based on the
                    // *response* (does it contain tool calls?), NOT on the
                    // MessageEnd stream event: every provider emits exactly one
                    // MessageEnd per generation regardless of whether it stopped
                    // to call a tool or stopped naturally.
                    if !this_generation_had_tool_calls {
                        break;
                    }
                    request = GenerateRequest {
                        messages: old_messages
                            .iter()
                            .chain(current_messages.iter())
                            .map(|m| m.data.clone())
                            .collect(),
                        system_prompt: system_prompt.clone(),
                        tool_specs: tool_specs.clone(),
                    };
                }

                let mut control_flow = AgentControlFlow::Output;
                for middleware in self.middlewares.iter() {
                    middleware
                        .after_generate(&generate_response, &mut control_flow)
                        .await?;
                }
                match control_flow {
                    AgentControlFlow::Output => break,
                    AgentControlFlow::GenerateWith(content) => {
                        current_messages.push(Message {
                            created_at: jiff::Timestamp::now(),
                            data: MessageType::MiddlewareMessage(content),
                            usage: None,
                        });
                        // Preserve the system prompt across the outer
                        // middleware-driven regeneration loop.
                        request = GenerateRequest {
                            messages: old_messages
                                .iter()
                                .chain(current_messages.iter())
                                .map(|m| m.data.clone())
                                .collect(),
                            system_prompt: Some(base_system_prompt.clone()),
                            ..Default::default()
                        };
                    }
                }
            }
            // The whole turn is done: every tool round settled and middleware
            // accepted the output. Emit a single TurnEnd so clients can release
            // their "sending" state. This is distinct from MessageEnd, which
            // only closes one provider generation and may be followed by more.
            Self::send(&sink, StreamEventData::TurnEnd)?;
            Ok(Turn {
                messages: current_messages.clone(),
                usage: total_usage.clone(),
                finished: true,
            })
        }
        .await;

        let outcome: Result<Turn, Turn> = match result {
            Ok(turn) => Ok(turn),
            Err(e) => {
                // Surface the error to the client as a terminal stream event,
                // then hand back the partial turn so the API layer can decide
                // whether to persist it (today it does not).
                let _ = sink.send(AgentEventType::StreamEvent(StreamEvent {
                    data: StreamEventData::MessageEnd(StopReason::Error(e.to_string())),
                    created_at: jiff::Timestamp::now(),
                }));
                Err(Turn {
                    messages: current_messages,
                    usage: total_usage,
                    finished: false,
                })
            }
        };
        // Cascade-abort every middleware's detached work first (so they stop
        // producing mev events — e.g. subagents get torn down), then abort the
        // merge relay. Runs on BOTH the Ok and Err exit paths, before run_loop
        // returns. Order matters: middlewares must quiesce before the relay
        // stops draining them.
        for middleware in self.middlewares.iter() {
            let _ = middleware.on_turn_end().await;
        }
        merge_relay.abort();
        outcome
    }

    /// Send a stream event as an [`crate::agent::AgentEvent`], allocating
    /// the index from the shared sink. A closed client channel fails the run.
    fn send(
        sink: &crate::agent::AgentEventSink,
        data: StreamEventData,
    ) -> Result<(), AgentError> {
        sink.send(AgentEventType::StreamEvent(StreamEvent {
            data,
            created_at: jiff::Timestamp::now(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::agent::test_mocks::{
        EchoMiddleware, InjectMiddleware, MockProvider, OneShotRegenerateMiddleware, text_msg,
        toolcall_msg,
    };
    use crate::extensions::Extensions;
    use crate::middleware::AgentControlFlow;

    static AGENT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Build an `Agent` whose `db` is a real (temp-file) toasty DB. `run_loop`
    /// never queries it, so the schema is never exercised — the handle only
    /// satisfies the struct field. Each call uses a unique path so parallel
    /// tests don't collide on a locked SQLite file.
    async fn make_agent(
        provider: Arc<dyn crate::provider::Provider>,
        middlewares: Vec<Box<dyn crate::middleware::Middleware>>,
    ) -> Agent {
        let n = AGENT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nekocode_runloop_test_{}_{n}.db",
            std::process::id()
        ));
        let db = nekocode_entities::prepare_db(path).await.expect("prepare_db");
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(middlewares),
            provider,
            extensions: Extensions::new(),
        }
    }

    fn text_input(s: &str) -> Vec<MessageContent> {
        vec![MessageContent::Text { content: s.into() }]
    }

    /// A simple successful run: one assistant text message, no tools. The
    /// returned turn carries the user + assistant messages, aggregate usage
    /// summed from the mock, and `finished = true`.
    #[tokio::test]
    async fn run_loop_success_returns_complete_turn() {
        let agent = make_agent(
            Arc::new(MockProvider::new(vec![text_msg("hello")])),
            vec![Box::new(EchoMiddleware)],
        )
        .await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let turn = agent
            .run_loop(
                text_input("hi"),
                Vec::new(),
                crate::agent::AgentEventSink::new(tx),
            )
            .await
            .expect("run should succeed");

        assert!(turn.finished, "finished flag should be true on success");
        assert_eq!(turn.messages.len(), 2, "user + one assistant message");
        assert!(matches!(
            turn.messages[0].data,
            MessageType::User(_)
        ));
        assert!(matches!(turn.messages[1].data, MessageType::Assistant(_)));
        // MockProvider reports 10 input / 5 output per call; one call here.
        assert_eq!(turn.usage.total_input, 10);
        assert_eq!(turn.usage.total_output, 5);
        // Assistant message carries per-message usage for persistence.
        assert!(turn.messages[1].usage.is_some());
    }

    /// A tool-call round: the first generation emits a tool call (registered
    /// by `EchoMiddleware`), the loop executes it and regenerates. The turn
    /// then contains user + assistant(toolcall) + toolresult + assistant(text).
    #[tokio::test]
    async fn run_loop_tool_call_round_appends_results() {
        let agent = make_agent(
            Arc::new(MockProvider::new(vec![
                toolcall_msg("call_1", "echo", serde_json::json!({"value": "v"})),
                text_msg("done"),
            ])),
            vec![Box::new(EchoMiddleware)],
        )
        .await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let turn = agent
            .run_loop(
                text_input("hi"),
                Vec::new(),
                crate::agent::AgentEventSink::new(tx),
            )
            .await
            .expect("run should succeed");

        assert!(turn.finished);
        assert_eq!(
            turn.messages.len(),
            4,
            "user + assistant(toolcall) + toolresult + assistant(text)"
        );
        assert!(matches!(turn.messages[2].data, MessageType::ToolCallResult(_)));
        // Two provider calls => 20 input / 10 output aggregated.
        assert_eq!(turn.usage.total_input, 20);
        assert_eq!(turn.usage.total_output, 10);
    }

    /// Error path: the mock exhausts its responses (returns an error). The run
    /// returns `Err(partial_turn)` with `finished = false`, and emits a
    /// `MessageEnd(Error)` stream event the caller can observe.
    #[tokio::test]
    async fn run_loop_error_returns_partial_turn_and_error_event() {
        let agent = make_agent(
            // No responses => first stream_generate errors ("mock exhausted").
            Arc::new(MockProvider::new(Vec::new())),
            vec![Box::new(EchoMiddleware)],
        )
        .await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let err = agent
            .run_loop(
                text_input("hi"),
                Vec::new(),
                crate::agent::AgentEventSink::new(tx),
            )
            .await
            .expect_err("run should fail");

        assert!(!err.finished, "partial turn must be unfinished");
        // The user message is always present; nothing else was produced.
        assert_eq!(err.messages.len(), 1);
        assert!(matches!(err.messages[0].data, MessageType::User(_)));

        // The error was surfaced as a MessageEnd(Error) stream event. Drain
        // buffered events (e.g. the agent's own MessageStart) until we find it.
        let mut saw_error_event = false;
        while let Ok(e) = rx.try_recv() {
            if matches!(
                e.data,
                AgentEventType::StreamEvent(StreamEvent {
                    data: StreamEventData::MessageEnd(StopReason::Error(_)),
                    ..
                })
            ) {
                saw_error_event = true;
                break;
            }
        }
        assert!(
            saw_error_event,
            "expected a MessageEnd(Error) stream event on failure"
        );
    }

    /// The outer middleware regeneration loop: `OneShotRegenerateMiddleware`
    /// injects a middleware message and forces one extra generation. The turn
    /// then contains the injected middleware message plus both assistant turns.
    #[tokio::test]
    async fn run_loop_middleware_regenerate_injects_message() {
        let agent = make_agent(
            Arc::new(MockProvider::new(vec![text_msg("first"), text_msg("second")])),
            vec![Box::new(OneShotRegenerateMiddleware {
                fired: std::sync::Mutex::new(false),
                inject: "regenerate me".into(),
            })],
        )
        .await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let turn = agent
            .run_loop(
                text_input("hi"),
                Vec::new(),
                crate::agent::AgentEventSink::new(tx),
            )
            .await
            .expect("run should succeed");

        assert!(turn.finished);
        // user + assistant(first) + middleware(inject) + assistant(second).
        assert_eq!(turn.messages.len(), 4);
        assert!(matches!(
            turn.messages[2].data,
            MessageType::MiddlewareMessage(_)
        ));
    }

    // Keep `InjectMiddleware` referenced (it's a public mock helper) and silence
    // the otherwise-unused import in configurations that don't exercise it.
    #[test]
    fn _inject_middleware_type_is_referenced() {
        let _ = InjectMiddleware(AgentControlFlow::Output);
    }

    /// The merge relay wraps each `MiddlewareEvent` (emitted by a middleware
    /// into `mev_tx`) into a uniquely-indexed `AgentEvent` on the parent
    /// stream, coexisting with the run's own `StreamEvent`s. The shared
    /// atomic index on the sink keeps all indices unique & contiguous.
    #[tokio::test]
    async fn merge_relay_forwards_middleware_event_with_unique_index() {
        use crate::agent::test_mocks::RelayMiddleware;
        use nekocode_types::generate::StreamEventData;

        let agent = make_agent(
            Arc::new(MockProvider::new(vec![text_msg("ok")])),
            vec![Box::new(RelayMiddleware)],
        )
        .await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = crate::agent::AgentEventSink::new(tx);
        let _turn = agent
            .run_loop(text_input("hi"), Vec::new(), sink)
            .await
            .expect("turn ok");

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        // The merge-relayed MiddlewareEvent must be present.
        let mev = events
            .iter()
            .find(|e| matches!(e.data, crate::agent::AgentEventType::MiddlewareEvent(_)))
            .expect("middleware event relayed");
        let _ = mev;
        // Indices across all events must be unique and contiguous 0..n.
        let mut idx: Vec<usize> = events.iter().map(|e| e.index).collect();
        idx.sort();
        let expected: Vec<usize> = (0..events.len()).collect();
        assert_eq!(idx, expected, "indices unique & contiguous");
        // And a StreamEvent (MessageEnd) coexists.
        assert!(events.iter().any(|e| matches!(
            e.data,
            crate::agent::AgentEventType::StreamEvent(ref se)
                if matches!(se.data, StreamEventData::MessageEnd(_))
        )));
    }
}
