pub mod error;
pub mod tool;
use std::sync::Arc;

use anyhow::anyhow;
use nekocode_entities::{message::Message, thread::Thread};
use nekocode_types::generate::{self, StreamEvent};
use serde::Serialize;
use toasty::{create, query};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    agent::{error::AgentError, tool::ToolRegistry},
    middleware::{AgentControlFlow, Middleware},
    provider::collect_db_messages,
    types::GenerateRequest,
};

#[derive(Clone, Serialize)]
pub enum AgentEvent {
    Message(Message),
    StreamEvent(StreamEvent),
}

#[derive(Serialize)]
pub struct RunLoopSummary {}

#[derive(Clone)]
pub struct Agent {
    thread_id: u64,
    db: toasty::Db,
    middlewares: Arc<Vec<Box<dyn Middleware>>>,
    provider: Arc<dyn crate::provider::Provider>,
    generate_option: crate::provider::GenerateOption,
}

impl Agent {
    pub fn new() -> Self {
        todo!()
    }

    pub async fn run_loop(
        &self,
        input: String,
        sender: UnboundedSender<AgentEvent>,
    ) -> Result<RunLoopSummary, AgentError> {
        let mut db = self.db.clone();
        let thread = query!(Thread FILTER .id == #(self.thread_id))
            .first()
            .exec(&mut db)
            .await?
            .ok_or(AgentError::ItemNotFound(format!(
                "Thread not found: {}",
                self.thread_id
            )))?;
        let generate_start_msg_id = if let Some(msg_id) = thread.generate_start_msg_id {
            msg_id
        } else {
            let msg = create!(Message {
                thread_id: self.thread_id,
                content: generate::Message::User(generate::MessageContent::Text(input)),
            })
            .exec(&mut db)
            .await?;
            let mut update = query!(Thread FILTER .id == #(self.thread_id)).update();
            update.set_generate_start_msg_id(msg.id);
            update.exec(&mut db).await?;
            msg.id
        };
        let msg = query!(Message FILTER .id == #generate_start_msg_id)
            .first()
            .exec(&mut db)
            .await?;
        let messages = if let Some(msg) = msg {
            query!(Message FILTER .id >= #(msg.id) AND .thread_id == #(self.thread_id) ORDER BY .created_at DESC)
                    .exec(&mut db)
                    .await?
        } else {
            return Err(AgentError::ItemNotFound(format!(
                "Message of Thread {} .generate_start_msg_id not found: {}",
                self.thread_id, generate_start_msg_id
            )));
        };
        let mut request = GenerateRequest {
            messages: collect_db_messages(messages),
            options: self.generate_option.clone(),
            ..Default::default()
        };
        loop {
            let mut tool_registry = ToolRegistry::new();
            for middleware in self.middlewares.iter() {
                middleware
                    .before_generate(&mut request, &mut tool_registry)
                    .await?;
            }
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let provider = self.provider.clone();
            let request_clone = request.clone();
            let handle =
                tokio::spawn(async move { provider.stream_generate(request_clone, tx).await });
            while let Some(event) = rx.recv().await {
                let agent_event = AgentEvent::StreamEvent((&event).into());
                sender
                    .send(agent_event)
                    .map_err(|e| AgentError::Other(anyhow!("error sending agent event {e}")))?;
            }
            let response = handle
                .await
                .map_err(|e| -> AgentError { anyhow!("error joining task {e}").into() })??;
            let mut control_flow = AgentControlFlow::Output;
            let generate_response = response.into();
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
