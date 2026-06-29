pub mod error;
pub mod new_agent;
#[cfg(test)]
pub(crate) mod test_mocks;
use std::{any::Any, sync::Arc};

use nekocode_types::generate::StreamEvent;
use serde::Serialize;

use crate::middleware::Middleware;

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

#[derive(Clone)]
pub struct Agent {
    pub thread_id: u64,
    /// Working directory used to build the agent's system prompt. Stored on
    /// the agent at construction time (from the loaded `Thread` row) so
    /// `run_loop` can format the prompt without a DB query.
    pub working_directory: String,
    pub db: toasty::Db,
    pub middlewares: Arc<Vec<Box<dyn Middleware>>>,
    pub provider: Arc<dyn crate::provider::Provider>,
    pub extensions: Arc<dashmap::DashMap<String, Box<dyn Any + Send + Sync>>>,
}

