use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolCallResult};

/// Tagged union for the kinds of messages that make up a conversation, serving
/// as both the on-disk (DB) and on-wire (stream) representation of a single
/// message.
///
/// `User` / `Assistant` carry the two ends of a generation; `MiddlewareMessage`
/// is a message injected by middleware into the conversation (e.g. a tool
/// result synthesized before re-invoking the provider); `ToolCallResult`
/// carries the outcome of executing an assistant tool call. The tag is keyed
/// by `"type"` so every variant serializes as a JSON object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "camelCase")]
pub enum MessageType {
    User(Vec<MessageContent>),
    Assistant(AssistantMessage),
    MiddlewareMessage(MessageContent),
    ToolCallResult(ToolCallResult),
}

/// A single timed, tagged message in a conversation. Wraps a [`MessageType`]
/// with its creation timestamp and (for assistant turns) token usage.
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

/// The assistant's side of a generation: an ordered list of content blocks,
/// which may interleave plain text (with optional reasoning) and tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub blocks: Vec<AssistantContentBlock>,
}

/// One block within an [`AssistantMessage`]. Text blocks carry the visible
/// content plus any chain-of-thought `reasoning_content` the provider emitted;
/// `ToolCall` is an in-flight call the agent must execute and feed back.
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

/// User/middleware-authored content. Currently a single text variant; kept
/// as an enum so richer content (images, etc.) can be added without churning
/// call sites.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum MessageContent {
    Text { content: String },
}

/// The payload of a streamed agent event, relayed live to the client. A
/// single provider generation produces `MessageStart` … (`Content` /
/// `ReasoningContent` / `ToolCall`)* … `MessageEnd`; the agent layer wraps the
/// whole multi-round turn with a single `TurnEnd`. Only serialized (sent over
/// the wire); the DB persists finished messages, not these deltas.
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

/// Why a single provider generation ended. `Stop` = finished naturally;
/// `Length` = hit the token cap; `Error` = the provider reported a failure.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Stop,
    Length,
    Error(String),
}

/// Metadata attached to a `MessageStart` event, identifying the role of the
/// message that is beginning to stream.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMetadata {
    pub role: Role,
}

/// Who authored a message. Mirrors [`MessageType`]'s variants: a `User` /
/// `Assistant` generation, or a `Middleware`-injected message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
    Middleware,
}

/// One timestamped event on the agent's live stream. `data` carries the
/// [`StreamEventData`] payload; `created_at` is when it was produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    pub data: StreamEventData,
    pub created_at: jiff::Timestamp,
}

/// Aggregated token usage for a message or turn. `cache_hit`/`cache_miss`
/// track prompt-caching behavior reported by the provider.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub total_input: usize,
    pub total_output: usize,
    pub cache_hit: bool,
    pub cache_miss: usize,
}

/// The in-memory result of running the agent for one user turn: the full
/// message transcript, aggregated usage, and whether the run finished
/// normally. Held in memory so the API layer can persist it as a `Turn` row.
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
