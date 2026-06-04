use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolCallResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    User(MessageContent),
    Assistant(AssistantMessage),
    MiddlewareMessage(MessageContent),
    ToolCallResult(ToolCallResult),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub blocks: Vec<AssistantContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssistantContentBlock {
    ToolCall(ToolCall),
    Text {
        content: String,
        reasoning_content: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageContent {
    Text(String),
}

#[derive(Debug, Clone, Serialize)]
pub enum StreamEventData {
    MessageStart(MessageMetadata),
    MessageEnd,
    Content(String),
    ReasoningContent(String),
    ToolCall(ToolCall),
    ToolCallResult(ToolCallResult),
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageMetadata {
    pub role: Role,
}

#[derive(Debug, Clone, Serialize)]
pub enum Role {
    User,
    Assistant,
    Middleware,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamEvent {
    pub data: StreamEventData,
    pub created_at: jiff::Timestamp,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub total_input: usize,
    pub total_output: usize,
    pub cache_hit: bool,
    pub cache_miss: usize,
}
