// ── Shared types matching the Rust backend API surface ──

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
  updatedAt: string
  createdAt: string
}

export interface Turn {
  id: number
  threadId: number
  turn_index: number
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

// ── Chat message types — serde(tag = "type") ──

export type ChatMessage =
  | { type: 'User' } & MessageContent
  | { type: 'Assistant' } & AssistantMessage
  | { type: 'MiddlewareMessage' } & MessageContent
  | { type: 'ToolCallResult' } & ToolCallResult

export type MessageContent = { Text: string }

export interface AssistantMessage {
  blocks: AssistantContentBlock[]
}

export type AssistantContentBlock =
  | { type: 'ToolCall' } & ToolCall
  | { type: 'Text'; content: string; reasoning_content: string | null }

// ── Tool types ──

export interface ToolCall {
  id: string
  name: string
  args: unknown
}

export interface ToolCallResult {
  id: string
  result: { type: 'success'; success: unknown } | { type: 'error'; error: string }
}

// ── Streaming types (serde default = externally tagged) ──

/** Raw stream event as deserialized from JSON. */
export type RawStreamEventData =
  | { MessageStart: MessageMetadata }
  | 'MessageEnd'
  | { Content: string }
  | { ReasoningContent: string }
  | { ToolCall: ToolCall }
  | { ToolCallResult: ToolCallResult }

/** Normalized stream event with a discriminator field. */
export type StreamEventData =
  | ({ type: 'MessageStart' } & MessageMetadata)
  | { type: 'MessageEnd' }
  | { type: 'Content'; text: string }
  | { type: 'ReasoningContent'; text: string }
  | ({ type: 'ToolCall' } & ToolCall)
  | ({ type: 'ToolCallResult' } & ToolCallResult)

export function normalizeStreamEvent(raw: RawStreamEventData): StreamEventData {
  if (raw === 'MessageEnd') return { type: 'MessageEnd' }
  if (typeof raw === 'object' && raw !== null) {
    if ('MessageStart' in raw) return { type: 'MessageStart', ...raw.MessageStart }
    if ('Content' in raw) return { type: 'Content', text: raw.Content }
    if ('ReasoningContent' in raw) return { type: 'ReasoningContent', text: raw.ReasoningContent }
    if ('ToolCall' in raw) return { type: 'ToolCall', ...raw.ToolCall }
    if ('ToolCallResult' in raw) return { type: 'ToolCallResult', ...raw.ToolCallResult }
  }
  throw new Error(`Unknown stream event: ${JSON.stringify(raw)}`)
}

export interface MessageMetadata {
  role: 'User' | 'Assistant' | 'Middleware'
}

export interface StreamEvent {
  data: RawStreamEventData
  created_at: string
}

// ── Agent event (sent through WebSocket) ──

export interface AgentEvent {
  index: number
  data: { StreamEvent: StreamEvent }
}

export type WebSocketEvent =
  | { Delta: AgentEvent }
  | { Stop: StopReason }

export interface StopReason {
  reason: 'Finished' | 'Interrupted' | 'Error'
  detail: unknown
}

// ── Usage ──

export interface Usage {
  total_input: number
  total_output: number
  cache_hit: boolean
  cache_miss: number
}

// ── Thread detail ──

export interface GetThreadResponse {
  id: number
  title: string | null
  working_directory: string
  updated_at: string
  created_at: string
  active: boolean
  generating: boolean
  turns: Turn[]
}
