use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatCompletionMessageParam>,
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
