pub mod error;
use std::sync::Arc;

use anyhow::anyhow;
use nekocode_entities::thread::Thread;
use nekocode_types::{
    generate::{AssistantContentBlock, MessageContent, StreamEvent, StreamEventData},
    tool::{ToolCallResult, ToolCallResultInner, ToolRegistry},
};
use serde::Serialize;
use toasty::{create, query};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    agent::error::AgentError,
    middleware::{AgentControlFlow, Middleware},
    provider::ProviderEvent,
    types::{GenerateRequest, GenerateResponse},
};

#[derive(Clone, Serialize)]
pub struct AgentEvent {
    pub index: usize,
    pub data: AgentEventType,
}

#[derive(Clone, Serialize)]
pub enum AgentEventType {
    StreamEvent(StreamEvent),
}

#[derive(Serialize)]
pub struct RunLoopSummary {}

#[derive(Clone)]
pub struct Agent {
    pub thread_id: u64,
    pub db: toasty::Db,
    pub middlewares: Arc<Vec<Box<dyn Middleware>>>,
    pub provider: Arc<dyn crate::provider::Provider>,
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
            query!(nekocode_entities::turn::Turn FILTER .id >= #turn_id AND .thread_id == #(self.thread_id) ORDER BY .created_at ASC)
                .include(nekocode_entities::turn::Turn::fields().messages())
                .exec(&mut db)
                .await?
        } else {
            Vec::new()
        };
        let mut this_turn = create!(nekocode_entities::turn::Turn {
            thread_id: self.thread_id,
            turn_index: old_turns.len() as u64,
            usage: Default::default(),
            finished: false,
        })
        .exec(&mut db)
        .await?;
        let user_message = create!(nekocode_entities::message::Message {
            turn_id: this_turn.id,
            message_index: 0,
            content: nekocode_types::generate::Message::User(MessageContent::Text(input)),
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
        let mut request = GenerateRequest {
            messages: messages.into_iter().map(|m| m.content).collect(),
            ..Default::default()
        };
        loop {
            let mut tool_registry = ToolRegistry::new();
            for middleware in self.middlewares.iter() {
                middleware
                    .before_generate(&mut request, &mut tool_registry)
                    .await?;
            }
            let mut generate_response = GenerateResponse::new();
            'tool_loop: loop {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let provider = self.provider.clone();
                let system_prompt = request.system_prompt.clone();
                let handle =
                    tokio::spawn(async move { provider.stream_generate(request, tx).await });
                while let Some(event) = (&mut rx).recv().await {
                    let agent_event = AgentEvent {
                        index,
                        data: AgentEventType::StreamEvent((&event).into()),
                    };
                    sender
                        .send(agent_event)
                        .map_err(|e| AgentError::Other(anyhow!("error sending agent event {e}")))?;
                    index += 1;
                    match event {
                        ProviderEvent::MessageEnd => break 'tool_loop,
                        _ => {}
                    }
                }
                let response = handle
                    .await
                    .map_err(|e| -> AgentError { anyhow!("error joining task {e}").into() })??;
                create!(nekocode_entities::message::Message {
                    turn_id: this_turn.id,
                    message_index: message_index,
                    content: nekocode_types::generate::Message::Assistant(response.message.clone()),
                })
                .exec(&mut db)
                .await?;
                message_index += 1;
                for block in response.message.blocks.iter() {
                    match block {
                        AssistantContentBlock::ToolCall(tool_call) => {
                            let tool_call_result = match tool_registry.get(&tool_call.name) {
                                Some(tool) => ToolCallResult {
                                    id: tool_call.id.clone(),
                                    result: ToolCallResultInner::from(
                                        tool.call(tool_call.args.clone()).await,
                                    ),
                                },
                                None => ToolCallResult {
                                    id: tool_call.id.clone(),
                                    result: ToolCallResultInner::Error("Tool not found".into()),
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
                this_turn = query!(nekocode_entities::turn::Turn FILTER .id == #(this_turn.id))
                    .include(nekocode_entities::turn::Turn::fields().messages())
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
                    messages: messages.into_iter().map(|m| m.content).collect(),
                    system_prompt: system_prompt,
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
                AgentControlFlow::GenerateWith(middleware_request) => {
                    request = middleware_request;
                    continue;
                }
            }
        }
        let mut update =
            query!(nekocode_entities::turn::Turn FILTER .id == #(this_turn.id)).update();
        update.set_finished(true);
        update.exec(&mut db).await?;

        Ok(RunLoopSummary {})
    }
}
