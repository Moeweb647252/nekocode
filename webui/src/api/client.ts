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
      'Content-Type': 'application/json',
      ...authHeaders(),
    },
  }
  if (body !== undefined) {
    opts.body = JSON.stringify(body)
  }
  const resp = await fetch(`${BASE_URL}${path}`, opts)
  if (!resp.ok) {
    throw new Error(`HTTP ${resp.status}: ${resp.statusText}`)
  }
  return resp.json() as Promise<ApiResponse<T>>
}

export function get<T>(path: string): Promise<ApiResponse<T>> {
  return request<T>('GET', path)
}

export function post<T>(path: string, body?: unknown): Promise<ApiResponse<T>> {
  return request<T>('POST', path, body)
}
