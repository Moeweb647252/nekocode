import { post } from './client'
import type { MiddlewareConfig, MiddlewareResponse } from './types'

export async function listMiddlewares(threadId: number): Promise<MiddlewareResponse[]> {
  const resp = await post<MiddlewareResponse[]>('/middleware/list', { threadId })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to list middlewares')
  return resp.data
}

export async function createMiddleware(
  threadId: number,
  name: string,
  config: MiddlewareConfig,
): Promise<MiddlewareResponse> {
  const resp = await post<MiddlewareResponse>('/middleware/create', { threadId, name, config })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to create middleware')
  return resp.data
}

export async function updateMiddleware(
  id: number,
  config?: MiddlewareConfig,
  enabled?: boolean,
): Promise<void> {
  const payload: Record<string, unknown> = { id }
  if (config !== undefined) payload.config = config
  if (enabled !== undefined) payload.enabled = enabled
  const resp = await post<null>('/middleware/update', payload)
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to update middleware')
}

export async function deleteMiddleware(id: number): Promise<void> {
  const resp = await post<null>('/middleware/delete', { id })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to delete middleware')
}