use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolCallResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    User(MessageContent),
    AssistantMessageStart,
    Assistant(MessageContent),
    AssistantReasoning(String),
    MiddlewareMessage(MessageContent),
    TooCall(ToolCall),
    ToolCallResult(ToolCallResult),
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
    pub created_at: u64,
}
