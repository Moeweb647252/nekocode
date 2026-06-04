pub mod error;
pub mod tool;
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
        let mut turns = if let Some(turn_id) = thread.generate_start_turn_id {
            query!(nekocode_entities::turn::Turn FILTER .id >= #turn_id AND .thread_id == #(self.thread_id) ORDER BY .created_at ASC)
                .include(nekocode_entities::turn::Turn::fields().messages())
                .exec(&mut db)
                .await?
        } else {
            Vec::new()
        };
        let new_turn = create!(nekocode_entities::turn::Turn {
            thread_id: self.thread_id,
            turn_index: turns.len() as u64,
            usage: Default::default(),
            finished: false,
        })
        .exec(&mut db)
        .await?;
        create!(nekocode_entities::message::Message {
            turn_id: new_turn.id,
            message_index: 0,
            content: nekocode_types::generate::Message::User(MessageContent::Text(input)),
        })
        .exec(&mut db)
        .await?;
        let mut message_index = 1;
        let turn_id = new_turn.id;
        turns.push(new_turn);
        let mut messages = Vec::new();
        for turn in turns.into_iter().rev() {
            let turn_messages =
                query!(nekocode_entities::message::Message FILTER .turn_id == #(turn.id))
                    .exec(&mut db)
                    .await?;
            messages.extend(turn_messages);
        }
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
                let request_clone = request.clone();
                let handle =
                    tokio::spawn(async move { provider.stream_generate(request_clone, tx).await });
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
                    turn_id: turn_id,
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
                                turn_id: turn_id,
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
            }

            let mut control_flow = AgentControlFlow::Output;
            for middleware in self.middlewares.iter() {
                middleware
                    .after_generate(&request, &generate_response, &mut control_flow)
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

        Ok(RunLoopSummary {})
    }
}
