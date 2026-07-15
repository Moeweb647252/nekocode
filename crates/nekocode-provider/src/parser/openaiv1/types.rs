// Variant names mirror OpenAI's wire-format type names verbatim
// (e.g. `ChatCompletion*` messages); renaming to satisfy clippy would
// diverge from the API surface we serialize against.
#![allow(clippy::enum_variant_names)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatCompletionMessageParam>,
    #[serde(default)]
    pub(crate) stream: bool,
    #[serde(default)]
    pub(crate) temperature: Option<f32>,
    #[serde(default)]
    pub(crate) top_p: Option<f32>,
    #[serde(default)]
    pub(crate) max_tokens: Option<usize>,
    #[serde(default)]
    pub(crate) stop: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) tools: Option<Vec<ChatCompletionTool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionTool {
    #[serde(rename = "type")]
    pub(crate) tool_type: String,
    pub(crate) function: ChatCompletionToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionToolFunction {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub(crate) enum ChatCompletionMessageParam {
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
pub(crate) struct ChatCompletionDeveloperMessageParam {
    pub(crate) content: String,
    pub(crate) name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionSystemMessageParam {
    pub(crate) content: String,
    pub(crate) name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionUserMessageParam {
    pub(crate) content: String,
    pub(crate) name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionAssistantMessageParam {
    pub(crate) content: String,
    pub(crate) name: Option<String>,
    pub(crate) refusal: Option<String>,
    pub(crate) tool_calls: Option<Vec<ChatCompletionMessageToolCall>>,
    pub(crate) prefix: Option<bool>,
    pub(crate) reasoning_content: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ChatCompletionMessageToolCall {
    #[serde(rename = "function")]
    ChatCompletionMessageFunctionToolCall(ChatCompletionMessageFunctionToolCall),
    #[serde(rename = "custom")]
    ChatCompletionMessageCustomToolCall(ChatCompletionMessageCustomToolCall),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionMessageFunctionToolCall {
    pub(crate) id: String,
    pub(crate) function: Function,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Function {
    pub(crate) arguments: String,
    pub(crate) name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionMessageCustomToolCall {
    pub(crate) id: String,
    pub(crate) custom: Custom,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Custom {
    pub(crate) input: String,
    pub(crate) name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionToolMessageParam {
    pub(crate) content: String,
    pub(crate) tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    System,
    User,
    Assistant,
    Tool,
    Developer,
}

// ── Streaming response types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionStreamResponse {
    pub(crate) id: String,
    pub(crate) object: ChatCompletionObject,
    pub(crate) created: u64,
    pub(crate) model: String,
    pub(crate) choices: Vec<ChatCompletionStreamChoice>,
    #[serde(default)]
    pub(crate) usage: Option<ChatCompletionStreamUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum ChatCompletionObject {
    #[serde(rename = "chat.completion.chunk")]
    ChatCompletionChunk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionStreamChoice {
    pub(crate) index: usize,
    pub(crate) delta: ChatCompletionStreamDelta,
    pub(crate) finish_reason: Option<FinishReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionStreamDelta {
    #[serde(default)]
    pub(crate) role: Option<Role>,
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<ChatCompletionStreamDeltaToolCall>>,
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionStreamDeltaToolCall {
    pub(crate) index: usize,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) function: Option<ChatCompletionStreamFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionStreamFunction {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatCompletionStreamUsage {
    pub(crate) prompt_tokens: usize,
    pub(crate) completion_tokens: usize,
    pub(crate) total_tokens: usize,
}
