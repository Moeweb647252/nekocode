use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolCallResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "camelCase")]
pub enum MessageType {
    User(Vec<MessageContent>),
    Assistant(AssistantMessage),
    MiddlewareMessage(MessageContent),
    ToolCallResult(ToolCallResult),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub created_at: jiff::Timestamp,
    pub data: MessageType,
    /// Per-message token usage (assistant messages only). Held in memory so
    /// the API layer can write it to the `Message.usage` DB column on persist.
    /// Skipped from serde so the on-disk/wire `content` shape stays the bare
    /// `MessageType` (see `nekocode-entities::message::Message.content`).
    #[serde(skip)]
    pub usage: Option<Usage>,
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

#[derive(Debug, Clone)]
pub struct Turn {
    pub messages: Vec<Message>,
    pub usage: Usage,
    /// `true` when the run completed normally; `false` when it was interrupted
    /// by an error (the `messages`/`usage` then reflect progress up to the
    /// failure point). Carried in-memory so the API layer can persist it as the
    /// `Turn.finished` DB column.
    pub finished: bool,
}
