use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessageRequest {
    pub content: Vec<MessageParam>,
    pub model: String,
    pub metadata: Option<Metadata>,
    pub ouput_config: Option<OutputConfig>,
    pub stop_sequences: Option<Vec<String>>,
    pub stream: bool,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub thinking: Option<ThinkingConfigParam>,
    pub tool_choice: Option<ToolChoice>,
    pub tools: Option<Vec<Tool>>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub max_tokens: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageParam {
    pub role: Role,
    pub content: MessageContentParam,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContentParam {
    String(String),
    Blocks(Vec<ContentBlockParam>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlockParam {
    #[serde(rename = "text")]
    TextBlockParam(TextBlockParam),
    #[serde(rename = "thinking")]
    ThinkingBlockParam { signature: String, thinking: String },
    #[serde(rename = "tool_use")]
    ToolUseBlockParam(ToolUseBlockParam),
    #[serde(rename = "tool_result")]
    ToolResultBlockParam(ToolResultBlockParam),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextBlockParam {
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseBlockParam {
    pub id: String,
    pub input: serde_json::Value,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultBlockParam {
    pub tool_use_id: String,
    pub content: Vec<ContentBlock>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThinkingConfigParam {
    #[serde(rename = "enabled")]
    Enabled {
        display: Option<ThinkingDisplayOption>,
    },
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "adaptive")]
    Adaptive {
        display: Option<ThinkingDisplayOption>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThinkingDisplayOption {
    #[serde(rename = "summarized")]
    Summarized,
    #[serde(rename = "omitted")]
    Omitted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolChoice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "tool")]
    Tool { name: String },
    #[serde(rename = "none")]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub effort: Option<OutputEffort>,
    pub format: Option<OutputConfigFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputEffort {
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    #[serde(rename = "max")]
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputConfigFormat {
    #[serde(rename = "json_schema")]
    JsonSchema { schema: serde_json::Value },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RawMessageStreamEvent {
    #[serde(rename = "message_start")]
    RawMessageStartEvent(RawMessageStartEvent),
    #[serde(rename = "message_delta")]
    RawMessageDeltaEvent(RawMessageDeltaEvent),
    #[serde(rename = "message_stop")]
    RawMessageStopEvent,
    #[serde(rename = "content_block_start")]
    RawContentBlockStartEvent(RawContentBlockStartEvent),
    #[serde(rename = "content_block_delta")]
    RawContentBlockDeltaEvent(RawContentBlockDeltaEvent),
    #[serde(rename = "content_block_stop")]
    RawContentBlockStopEvent(RawContentBlockStopEvent),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMessageStartEvent {
    pub message: Message,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMessageDeltaEvent {}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawContentBlockStartEvent {
    pub index: usize,
    pub block: ContentBlock,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawContentBlockDeltaEvent {
    pub index: usize,
    pub delta: RawContentBlockDelta,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawContentBlockStopEvent {
    pub index: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RawContentBlockDelta {
    #[serde(rename = "text_delta")]
    TextDelta(TextDelta),
    #[serde(rename = "input_json_delta")]
    InputJsonDelta(InputJsonDelta),
    #[serde(rename = "thinking_delta")]
    ThinkingDelta(ThinkingDelta),
    #[serde(rename = "signature_delta")]
    SignatureDelta(SignatureDelta),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextDelta {
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputJsonDelta {
    pub partial_json: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingDelta {
    pub thinking: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureDelta {
    pub signature: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StopReason {
    #[serde(rename = "end_turn")]
    EndTurn,
    #[serde(rename = "max_tokens")]
    MaxTokens,
    #[serde(rename = "stop_sequence")]
    StopSequence,
    #[serde(rename = "tool_use")]
    ToolUse,
    #[serde(rename = "pause_turn")]
    PauseTurn,
    #[serde(rename = "refusal")]
    Refusal,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheCreation {
    pub ephemeral_1h_input_tokens: usize,
    pub ephemeral_5m_input_tokens: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    pub thinking_tokens: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub cache_creation: CacheCreation,
    pub cache_creation_input_tokens: usize,
    pub cache_read_input_tokens: usize,
    pub input_tokens: usize,
    pub output_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub role: Role,
    pub stop_reason: StopReason,
    pub usage: Usage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    TextBlock { text: String },
    #[serde(rename = "tinking")]
    ThinkingBlock { signature: String, thinking: String },
    #[serde(rename = "tool_use")]
    ToolUseBlock {
        id: String,
        input: serde_json::Value,
        name: String,
    },
}
