// ── Shared types matching the Rust backend (serde tag = "type", rename_all = "camelCase") ──

export interface ApiResponse<T = unknown> {
  code: string
  data: T
  msg: string | null
}

// ── Entity models ──

export interface Thread {
  id: number
  title: string | null
  workingDirectory: string
  model: string
  generateStartTurnId: number | null
  workspaceId: number | null
  updatedAt: string
  createdAt: string
}

// A workspace owns the threads that share a working directory. `name` is an
// optional display label (defaults to the directory basename); `threads` is
// materialized by the /workspace/list and /workspace/get endpoints.
export interface WorkspaceResponse {
  id: number
  workingDirectory: string
  name: string | null
  updatedAt: string
  createdAt: string
  threads: Thread[]
}

export interface Turn {
  id: number
  threadId: number
  turnIndex: number
  usage: Usage
  finished: boolean
  updatedAt: string
  createdAt: string
  messages?: Message[]
}

export interface Message {
  id: number
  turnId: number
  messageIndex: number
  content: ChatMessage
  usage: Usage | null
  updatedAt: string
  createdAt: string
}

export interface Token {
  id: number
  token: string
  expiresAt: string
}

// ════════════════════════════════════════════════════════════════════
// Chat message types — serde(tag = "type", content = "data", rename_all = "camelCase")
// ════════════════════════════════════════════════════════════════════

/** Internally tagged enum variant for MessageContent. */
export interface TextContent {
  type: 'text'
  content: string
}

/** User message data: MessageContent serialized per its own serde rules. */
export type UserMessageData = TextContent

/** Tool call result data: ToolCallResult struct flattened. */
export interface ToolCallResultData {
  id: string
  result: ToolCallResultInner
}

export type ChatMessage =
  | { type: 'user'; data: UserMessageData }
  | { type: 'assistant'; data: AssistantMessage }
  | { type: 'middlewareMessage'; data: UserMessageData }
  | { type: 'toolCallResult'; data: ToolCallResultData }

/** Internally tagged: serde(tag = "type", rename_all = "camelCase"). */
export type AssistantBlock =
  | ({ type: 'toolCall' } & ToolCall)
  | { type: 'text'; content: string; reasoningContent: string | null }

export interface AssistantMessage {
  blocks: AssistantBlock[]
}

// ── Tool types — serde(rename_all = "camelCase") ──

export interface ToolCall {
  id: string
  name: string
  args: unknown
}

export interface ToolCallResult {
  id: string
  result: ToolCallResultInner
}

/** serde(tag = "type") with explicit rename. */
export type ToolCallResultInner =
  | { type: 'success'; value: unknown }
  | { type: 'error'; error: string }

// ════════════════════════════════════════════════════════════════════
// Streaming types — serde(tag = "type", content = "data", rename_all = "camelCase")
// ════════════════════════════════════════════════════════════════════

export type RawStreamEventData =
  | { type: 'messageStart'; data: MessageMetadata }
  | { type: 'messageEnd' }
  | { type: 'turnEnd' }
  | { type: 'content'; data: string }
  | { type: 'reasoningContent'; data: string }
  | { type: 'toolCall'; data: ToolCall }
  | { type: 'toolCallResult'; data: ToolCallResult }

export interface MessageMetadata {
  role: 'user' | 'assistant' | 'middleware'
}

export interface StreamEvent {
  data: RawStreamEventData
  createdAt: string
}

// ── Agent event — serde(rename_all = "camelCase"), AgentEventType serde(tag = "type") ──

export interface AgentEvent {
  index: number
  data: AgentEventType
}

export type AgentEventType = { type: 'streamEvent'; data: RawStreamEventData; createdAt: string }

// ── WebSocket event — serde(rename_all = "camelCase"), externally tagged ──

export type WebSocketEvent =
  | { delta: AgentEvent }
  | { stop: StopReason }

export interface StopReason {
  reason: 'finished' | 'interrupted' | 'error'
  detail: unknown
}

// ── Usage — serde(rename_all = "camelCase") ──

export interface Usage {
  totalInput: number
  totalOutput: number
  cacheHit: boolean
  cacheMiss: number
}

// ── Thread detail — serde(rename_all = "camelCase") ──

export interface ThreadResponse {
  id: number
  title: string | null
  workingDirectory: string
  model: string
  updatedAt: string
  createdAt: string
  active: boolean
  generating: boolean
  turns: Turn[]
}

export interface MiddlewareResponse {
  id: number
  name: string
  config: Record<string, unknown>
  enabled: boolean
}
