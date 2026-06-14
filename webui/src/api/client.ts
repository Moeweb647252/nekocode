import { useAppStore } from '@/stores/app'
import type { ApiResponse } from './types'

const BASE_URL = '/api'

function authHeaders(): Record<string, string> {
  const store = useAppStore()
  if (store.token) {
    return { Token: store.token }
  }
  return {}
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<ApiResponse<T>> {
  const opts: RequestInit = {
    method,
    headers: {
      ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}),
      ...authHeaders(),
    },
  }
  if (body !== undefined) {
    opts.body = JSON.stringify(body)
  }
  const resp = await fetch(`${BASE_URL}${path}`, opts)
  if (!resp.ok) {
    // Surface the backend's JSON error body when available so callers can show
    // a meaningful message; fall back to the HTTP status text for non-JSON
    // bodies (e.g. a proxy 502 HTML page).
    let detail = `${resp.status} ${resp.statusText}`
    const text = await resp.text().catch(() => '')
    if (text) {
      try {
        const parsed = JSON.parse(text) as ApiResponse
        if (parsed.msg) detail = parsed.msg
        else if (parsed.code) detail = `${parsed.code} (${resp.status})`
      } catch {
        detail = text.slice(0, 200)
      }
    }
    throw new Error(detail)
  }
  return resp.json() as Promise<ApiResponse<T>>
}

export function get<T>(path: string): Promise<ApiResponse<T>> {
  return request<T>('GET', path)
}

export function post<T>(path: string, body?: unknown): Promise<ApiResponse<T>> {
  return request<T>('POST', path, body)
}
