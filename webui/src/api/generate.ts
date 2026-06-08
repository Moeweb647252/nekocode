import { useAppStore } from '@/stores/app'
import type { AgentEvent, StopReason, WebSocketEvent } from './types'

function wsUrl(path: string): string {
  const { protocol, host } = window.location
  const wsProtocol = protocol === 'https:' ? 'wss:' : 'ws:'
  return `${wsProtocol}//${host}/api/generate${path}`
}

function authToken(): string | undefined {
  return useAppStore().token
}

// ── Callbacks ──

export interface GenerateCallbacks {
  onDelta(event: AgentEvent): void
  onStop(reason: StopReason): void
  onError(err: Error): void
}

// ── Main generation stream ──

/**
 * Open a WebSocket to start a new generation on a thread.
 * Returns a cleanup function to close the socket.
 */
export function streamGenerate(
  threadId: number,
  userInput: string,
  callbacks: GenerateCallbacks,
): () => void {
  const url = wsUrl('/stream')
  return connectStream(url, callbacks, (ws) => {
    ws.send(JSON.stringify({ user_input: userInput, thread_id: threadId }))
  })
}

// ── Watch an existing generation ──

/**
 * Open a WebSocket to watch an already-running generation.
 * Returns a cleanup function to close the socket.
 */
export function watchStream(
  threadId: number,
  callbacks: GenerateCallbacks,
): () => void {
  const url = wsUrl(`/watch/${threadId}`)
  return connectStream(url, callbacks)
}

// ── Internal ──

function connectStream(
  url: string,
  callbacks: GenerateCallbacks,
  onOpen?: (ws: WebSocket) => void,
): () => void {
  const token = authToken()
  const protocols = token ? [token] : undefined
  const ws = new WebSocket(url, protocols)

  ws.addEventListener('open', () => {
    onOpen?.(ws)
  })

  ws.addEventListener('message', (event: MessageEvent<string>) => {
    try {
      const msg: WebSocketEvent = JSON.parse(event.data)
      if ('Delta' in msg) {
        callbacks.onDelta(msg.Delta)
      } else if ('Stop' in msg) {
        callbacks.onStop(msg.Stop)
        ws.close()
      }
    } catch (err) {
      callbacks.onError(err instanceof Error ? err : new Error(String(err)))
    }
  })

  ws.addEventListener('error', () => {
    callbacks.onError(new Error('WebSocket connection error'))
  })

  ws.addEventListener('close', (event) => {
    if (!event.wasClean) {
      callbacks.onError(new Error(`WebSocket closed unexpectedly (code ${event.code})`))
    }
  })

  return () => {
    if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
      ws.close(1000, 'client disconnect')
    }
  }
}
