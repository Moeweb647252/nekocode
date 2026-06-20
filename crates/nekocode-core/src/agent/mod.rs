pub mod error;
pub mod subagent;
use std::{any::Any, sync::Arc};

use anyhow::anyhow;
use nekocode_entities::{thread::Thread, turn::Turn};
use nekocode_types::{
    generate::{
        AssistantContentBlock, MessageContent, MessageMetadata, Role, StreamEvent, StreamEventData,
        Usage,
    },
    tool::{ToolCallResult, ToolCallResultInner, ToolRegistry},
};
use serde::Serialize;
use toasty::{Json, create, query};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    agent::error::AgentError,
    middleware::{AgentControlFlow, Middleware},
    provider::ProviderEvent,
    types::{GenerateRequest, GenerateResponse},
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub index: usize,
    pub data: AgentEventType,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum AgentEventType {
    StreamEvent(StreamEvent),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLoopSummary {
    /// Aggregate token usage across every provider generation in this run.
    pub usage: Usage,
}

#[derive(Clone)]
pub struct Agent {
    pub thread_id: u64,
    pub db: toasty::Db,
    pub middlewares: Arc<Vec<Box<dyn Middleware>>>,
    pub provider: Arc<dyn crate::provider::Provider>,
    pub extensions: Arc<dashmap::DashMap<String, Box<dyn Any + Send + Sync>>>,
}

impl Agent {
    pub async fn run_loop(
        &self,
        input: String,
        sender: UnboundedSender<AgentEvent>,
    ) -> Result<RunLoopSummary, AgentError> {
        let mut index = 0;
        let mut db = self.db.clone();
        let thread = query!(Thread FILTER .id == #(self.thread_id))
            .first()
            .exec(&mut db)
            .await?
            .ok_or(AgentError::ItemNotFound(format!(
                "Thread not found: {}",
                self.thread_id
            )))?;
        let old_turns = if let Some(turn_id) = thread.generate_start_turn_id {
            query!(Turn FILTER .id >= #turn_id AND .thread_id == #(self.thread_id) ORDER BY .created_at ASC)
                .include(Turn::fields().messages())
                .exec(&mut db)
                .await?
        } else {
            query!(Turn FILTER .thread_id == #(self.thread_id) ORDER BY .created_at ASC)
                .include(Turn::fields().messages())
                .exec(&mut db)
                .await?
        };
        let mut this_turn = create!(Turn {
            thread_id: self.thread_id,
            turn_index: old_turns.len() as u64,
            usage: Json(Default::default()),
            finished: false,
        })
        .exec(&mut db)
        .await?;
        let user_message = create!(nekocode_entities::message::Message {
            turn_id: this_turn.id,
            message_index: 0,
            content: nekocode_types::generate::Message::User(MessageContent::Text {
                content: input,
            }),
        })
        .exec(&mut db)
        .await?;
        let mut message_index = 1;
        let mut old_messages = Vec::new();
        for turn in old_turns.into_iter() {
            let turn_messages = turn.messages.get().to_owned();
            old_messages.extend(turn_messages);
        }
        let mut messages = old_messages.clone();
        messages.push(user_message);
        // Capture the system prompt up front so it survives both the inner
        // (tool-call) and outer (middleware) regeneration loops.
        let base_system_prompt = format!("Working directory: {}\n", thread.working_directory);
        let mut request = GenerateRequest {
            messages: messages.into_iter().map(|m| m.content.0).collect(),
            system_prompt: Some(base_system_prompt.clone()),
            ..Default::default()
        };
        // Accumulated usage across all provider calls in this run.
        let mut total_usage = Usage::default();
        loop {
            let mut tool_registry = ToolRegistry::new();
            for middleware in self.middlewares.iter() {
                middleware
                    .before_generate(&mut request, &mut tool_registry)
                    .await?;
            }
            request.tool_specs = tool_registry.specs();
            let system_prompt = request.system_prompt.clone();
            let tool_specs = request.tool_specs.clone();
            let mut generate_response = GenerateResponse::new();
            loop {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let provider = self.provider.clone();
                sender
                    .send(AgentEvent {
                        index,
                        data: AgentEventType::StreamEvent(StreamEvent {
                            data: StreamEventData::MessageStart(MessageMetadata {
                                role: Role::Assistant,
                            }),
                            created_at: jiff::Timestamp::now(),
                        }),
                    })
                    .map_err(|e| AgentError::Other(anyhow!("error sending agent event {e}")))?;
                index += 1;
                let handle =
                    tokio::spawn(async move { provider.stream_generate(request, tx).await });
                // Whether this generation requested any tool calls. The break
                // decision is based on the *response* (does it contain tool
                // calls?), NOT on the MessageEnd stream event: every provider
                // emits exactly one MessageEnd per generation regardless of
                // whether it stopped to call a tool or stopped naturally.
                while let Some(event) = (&mut rx).recv().await {
                    // The agent emits its own MessageStart above before the
                    // provider call; skip the provider's duplicate so the
                    // client doesn't see two MessageStart events per turn.
                    if matches!(event, ProviderEvent::MessageStart) {
                        continue;
                    }
                    let agent_event = AgentEvent {
                        index,
                        data: AgentEventType::StreamEvent((&event).into()),
                    };
                    sender
                        .send(agent_event)
                        .map_err(|e| AgentError::Other(anyhow!("error sending agent event {e}")))?;
                    index += 1;
                }
                let response = handle
                    .await
                    .map_err(|e| -> AgentError { anyhow!("error joining task {e}").into() })??;
                // Accumulate usage from this provider call.
                total_usage.total_input += response.usage.total_input;
                total_usage.total_output += response.usage.total_output;
                total_usage.cache_miss += response.usage.cache_miss;
                if response.usage.cache_hit {
                    total_usage.cache_hit = true;
                }
                let assistant_usage = Json(response.usage.clone());
                create!(nekocode_entities::message::Message {
                    turn_id: this_turn.id,
                    message_index: message_index,
                    content: nekocode_types::generate::Message::Assistant(response.message.clone()),
                    usage: Some(assistant_usage),
                })
                .exec(&mut db)
                .await?;
                message_index += 1;
                let mut this_generation_had_tool_calls = false;
                for block in response.message.blocks.iter() {
                    match block {
                        AssistantContentBlock::ToolCall(tool_call) => {
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
                            create!(nekocode_entities::message::Message {
                                turn_id: this_turn.id,
                                message_index: message_index,
                                content: nekocode_types::generate::Message::ToolCallResult(
                                    tool_call_result.clone()
                                ),
                            })
                            .exec(&mut db)
                            .await?;
                            message_index += 1;
                            let stream_event = StreamEvent {
                                data: StreamEventData::ToolCallResult(tool_call_result),
                                created_at: jiff::Timestamp::now(),
                            };
                            let agent_event = AgentEvent {
                                index,
                                data: AgentEventType::StreamEvent(stream_event.clone()),
                            };
                            sender.send(agent_event).map_err(|e| {
                                AgentError::Other(anyhow!("error sending agent event {e}"))
                            })?;
                            index += 1;
                            generate_response.merge_stream_event(stream_event);
                        }
                        _ => {}
                    }
                }
                generate_response.merge(response);
                // Only break out of the inner (tool-call) loop when this
                // generation finished naturally. If it emitted tool calls
                // we've already executed them and persisted the results above,
                // so loop again to feed those results back into a fresh
                // generation. The decision is based on the *response*, NOT on
                // the MessageEnd stream event: every provider emits exactly
                // one MessageEnd per generation regardless of whether it
                // stopped to call a tool or stopped naturally.
                let has_tool_calls = this_generation_had_tool_calls;
                if !has_tool_calls {
                    break;
                }
                this_turn = query!(Turn FILTER .id == #(this_turn.id))
                    .include(Turn::fields().messages())
                    .first()
                    .exec(&mut db)
                    .await?
                    .ok_or(AgentError::ItemNotFound(format!(
                        "Turn not found: {}",
                        this_turn.id
                    )))?;
                let mut messages = old_messages.clone();
                messages.extend(this_turn.messages.get().to_owned());
                request = GenerateRequest {
                    messages: messages.into_iter().map(|m| m.content.0).collect(),
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
                    toasty::create!(nekocode_entities::message::Message {
                        turn_id: this_turn.id,
                        message_index: message_index,
                        content: nekocode_types::generate::Message::MiddlewareMessage(content),
                    })
                    .exec(&mut db)
                    .await?;
                    let mut messages = old_messages.clone();
                    messages.extend(this_turn.messages.get().to_owned());
                    // Preserve the system prompt and tool specs across the
                    // outer middleware-driven regeneration loop.
                    request = GenerateRequest {
                        messages: messages.into_iter().map(|m| m.content.0).collect(),
                        system_prompt: Some(base_system_prompt.clone()),
                        ..Default::default()
                    };
                }
            }
        }
        // The whole turn is done: every tool round is settled and middleware
        // accepted the output. Emit a single TurnEnd so clients can release
        // their "sending" state. This is distinct from MessageEnd, which only
        // closes one provider generation and may be followed by more rounds.
        sender
            .send(AgentEvent {
                index,
                data: AgentEventType::StreamEvent(StreamEvent {
                    data: StreamEventData::TurnEnd,
                    created_at: jiff::Timestamp::now(),
                }),
            })
            .map_err(|e| AgentError::Other(anyhow!("error sending agent event {e}")))?;
        let mut update = query!(Turn FILTER .id == #(this_turn.id)).update();
        update.set_finished(true);
        update.set_usage(Json(total_usage.clone()));
        update.exec(&mut db).await?;

        Ok(RunLoopSummary { usage: total_usage })
    }
}
