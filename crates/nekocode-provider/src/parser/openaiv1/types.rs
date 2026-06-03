use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatCompletionMessageParam>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub stop: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum ChatCompletionMessageParam {
    #[serde(rename = "developer")]
    ChatCompletionDeveloperMessageParam(ChatCompletionDeveloperMessageParam),
    #[serde(rename = "system")]
    ChatCompletionSystemMessageParam(ChatCompletionSystemMessageParam),
    #[serde(rename = "user")]
    ChatCompletionUserMessageParam(ChatCompletionUserMessageParam),
    #[serde(rename = "assistant")]
    ChatCompletionAssistantMessageParam(ChatCompletionAssistantMessageParam),
    #[serde(rename = "tool")]
    ChatCompletionToolMessageParam(ChatCompletionToolMessageParam),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionDeveloperMessageParam {
    pub content: String,
    pub name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionSystemMessageParam {
    pub content: String,
    pub name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionUserMessageParam {
    pub content: String,
    pub name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionAssistantMessageParam {
    pub content: String,
    pub name: Option<String>,
    pub refusal: Option<String>,
    pub tool_calls: Option<Vec<ChatCompletionMessageToolCall>>,
    pub prefix: Option<bool>,
    pub reasoning_content: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChatCompletionMessageToolCall {
    #[serde(rename = "function")]
    ChatCompletionMessageFunctionToolCall(ChatCompletionMessageFunctionToolCall),
    #[serde(rename = "custom")]
    ChatCompletionMessageCustomToolCall(ChatCompletionMessageCustomToolCall),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionMessageFunctionToolCall {
    pub name: String,
    pub arguments: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionMessageCustomToolCall {
    pub name: String,
    pub input: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionToolMessageParam {
    pub content: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
    Developer,
}

// ── Streaming response types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionStreamResponse {
    pub id: String,
    pub object: ChatCompletionObject,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatCompletionStreamChoice>,
    #[serde(default)]
    pub usage: Option<ChatCompletionStreamUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "object")]
pub enum ChatCompletionObject {
    #[serde(rename = "chat.completion.chunk")]
    ChatCompletionChunk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionStreamChoice {
    pub index: usize,
    pub delta: ChatCompletionStreamDelta,
    pub finish_reason: Option<FinishReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionStreamDelta {
    #[serde(default)]
    pub role: Option<Role>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatCompletionStreamDeltaToolCall>>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionStreamDeltaToolCall {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<ChatCompletionStreamFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionStreamFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionStreamUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}
