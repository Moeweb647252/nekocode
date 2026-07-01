pub mod error;
pub mod new_agent;
pub mod sink;
#[cfg(test)]
pub(crate) mod test_mocks;
use std::borrow::Cow;
use std::sync::Arc;

use nekocode_types::generate::StreamEvent;
use serde::Serialize;

use crate::extensions::Extensions;
use crate::middleware::Middleware;

pub use sink::AgentEventSink;

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
    /// An event relayed out of a child generation by a middleware
    /// (subagent today; reusable by subthread / others later). The
    /// payload is an opaque JSON value + a source-published type tag,
    /// so this enum never has to know the internal shape of each
    /// source's events.
    MiddlewareEvent(MiddlewareEvent),
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiddlewareEvent {
    /// Originating middleware kind, e.g. "subagent".
    pub source: Cow<'static, str>,
    /// Stable id of the originating child (subagent agent_id).
    pub source_id: u64,
    /// Source-published type tag for `data`, e.g. "agentEvent".
    pub event_type: String,
    /// Opaque payload. For subagent: the serialized child `AgentEvent`.
    pub data: serde_json::Value,
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
    pub extensions: Extensions,
}

#[cfg(test)]
mod middleware_event_tests {
    use super::*;
    use nekocode_types::generate::{StreamEvent, StreamEventData};

    #[test]
    fn middleware_event_serializes_camel_case() {
        let child = AgentEvent {
            index: 7,
            data: AgentEventType::StreamEvent(StreamEvent {
                data: StreamEventData::TurnEnd,
                created_at: jiff::Timestamp::now(),
            }),
        };
        let mev = MiddlewareEvent {
            source: Cow::Borrowed("subagent"),
            source_id: 42,
            event_type: "agentEvent".into(),
            data: serde_json::to_value(&child).unwrap(),
        };
        let wrapped = AgentEvent {
            index: 3,
            data: AgentEventType::MiddlewareEvent(mev),
        };
        let json = serde_json::to_value(&wrapped).unwrap();
        assert_eq!(json["index"], 3);
        assert_eq!(json["data"]["type"], "middlewareEvent");
        assert_eq!(json["data"]["source"], "subagent");
        assert_eq!(json["data"]["sourceId"], 42);
        assert_eq!(json["data"]["eventType"], "agentEvent");
        assert_eq!(json["data"]["data"]["index"], 7);
        // The nested child AgentEvent's own enum tag survives the opaque
        // payload path (one level deeper than the child's `index`).
        assert_eq!(json["data"]["data"]["data"]["type"], "streamEvent");
    }
}

