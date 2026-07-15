// Variant names mirror Anthropic's wire-format type names verbatim
// (e.g. `Raw*` events, `*BlockParam` blocks); renaming to satisfy clippy
// would diverge from the API surface we serialize against.
#![allow(clippy::enum_variant_names)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CreateMessageRequest {
    pub(crate) content: Vec<MessageParam>,
    pub(crate) model: String,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) output_config: Option<OutputConfig>,
    pub(crate) stop_sequences: Option<Vec<String>>,
    pub(crate) stream: bool,
    pub(crate) system: Option<String>,
    pub(crate) temperature: Option<f32>,
    pub(crate) thinking: Option<ThinkingConfigParam>,
    pub(crate) tool_choice: Option<ToolChoice>,
    pub(crate) tools: Option<Vec<Tool>>,
    pub(crate) top_p: Option<f32>,
    pub(crate) top_k: Option<u32>,
    pub(crate) max_tokens: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MessageParam {
    pub(crate) role: Role,
    pub(crate) content: MessageContentParam,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum MessageContentParam {
    String(String),
    Blocks(Vec<ContentBlockParam>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Role {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ContentBlockParam {
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
pub(crate) struct TextBlockParam {
    pub(crate) text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolUseBlockParam {
    pub(crate) id: String,
    pub(crate) input: serde_json::Value,
    pub(crate) name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolResultBlockParam {
    pub(crate) tool_use_id: String,
    pub(crate) content: Vec<ContentBlock>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum ThinkingConfigParam {
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
pub(crate) enum ThinkingDisplayOption {
    #[serde(rename = "summarized")]
    Summarized,
    #[serde(rename = "omitted")]
    Omitted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum ToolChoice {
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
pub(crate) struct Tool {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Metadata {
    pub(crate) user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OutputConfig {
    pub(crate) effort: Option<OutputEffort>,
    pub(crate) format: Option<OutputConfigFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum OutputEffort {
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
pub(crate) enum OutputConfigFormat {
    #[serde(rename = "json_schema")]
    JsonSchema { schema: serde_json::Value },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum RawMessageStreamEvent {
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
pub(crate) struct RawMessageStartEvent {
    pub(crate) message: Message,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RawMessageDeltaEvent {
    #[serde(default)]
    pub(crate) delta: RawMessageDelta,
    #[serde(default)]
    pub(crate) usage: Option<DeltaUsage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RawMessageDelta {
    #[serde(default)]
    pub(crate) stop_reason: Option<StopReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeltaUsage {
    #[serde(default)]
    pub(crate) output_tokens: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RawContentBlockStartEvent {
    pub(crate) index: usize,
    pub(crate) block: ContentBlock,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RawContentBlockDeltaEvent {
    pub(crate) index: usize,
    pub(crate) delta: RawContentBlockDelta,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RawContentBlockStopEvent {
    pub(crate) index: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum RawContentBlockDelta {
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
pub(crate) struct TextDelta {
    pub(crate) text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InputJsonDelta {
    pub(crate) partial_json: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ThinkingDelta {
    pub(crate) thinking: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SignatureDelta {
    pub(crate) signature: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StopReason {
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CacheCreation {
    #[serde(default)]
    pub(crate) ephemeral_1h_input_tokens: usize,
    #[serde(default)]
    pub(crate) ephemeral_5m_input_tokens: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Usage {
    #[serde(default)]
    pub(crate) cache_creation: CacheCreation,
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: usize,
    #[serde(default)]
    pub(crate) cache_read_input_tokens: usize,
    #[serde(default)]
    pub(crate) input_tokens: usize,
    #[serde(default)]
    pub(crate) output_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Message {
    pub(crate) id: String,
    pub(crate) content: Vec<ContentBlock>,
    pub(crate) role: Role,
    pub(crate) stop_reason: StopReason,
    pub(crate) usage: Usage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ContentBlock {
    #[serde(rename = "text")]
    TextBlock { text: String },
    #[serde(rename = "thinking")]
    ThinkingBlock { signature: String, thinking: String },
    #[serde(rename = "tool_use")]
    ToolUseBlock {
        id: String,
        input: serde_json::Value,
        name: String,
    },
}
