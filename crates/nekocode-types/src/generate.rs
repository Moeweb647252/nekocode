use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolCallResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "camelCase")]
pub enum Message {
    User(MessageContent),
    Assistant(AssistantMessage),
    MiddlewareMessage(MessageContent),
    ToolCallResult(ToolCallResult),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub blocks: Vec<AssistantContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum AssistantContentBlock {
    ToolCall(ToolCall),
    #[serde(rename_all = "camelCase")]
    Text {
        content: String,
        reasoning_content: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum MessageContent {
    Text { content: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type", content = "data")]
pub enum StreamEventData {
    MessageStart(MessageMetadata),
    MessageEnd(StopReason),
    /// Signals the end of the whole agent turn (every tool round done, final
    /// answer settled). Distinct from `MessageEnd`, which only marks the
    /// boundary of a single provider generation.
    TurnEnd,
    Content(String),
    ReasoningContent(String),
    ToolCall(ToolCall),
    ToolCallResult(ToolCallResult),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Stop,
    Length,
    Error(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMetadata {
    pub role: Role,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
    Middleware,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    pub data: StreamEventData,
    pub created_at: jiff::Timestamp,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub total_input: usize,
    pub total_output: usize,
    pub cache_hit: bool,
    pub cache_miss: usize,
}
