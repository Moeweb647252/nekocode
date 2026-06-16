import { post } from './client'
import type { Middleware } from './types'

export async function listMiddlewares(threadId: number): Promise<Middleware[]> {
  const resp = await post<Middleware[]>('/middleware/list', { threadId })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to list middlewares')
  return resp.data
}

export async function updateMiddleware(
  id: number,
  config: Record<string, unknown>,
): Promise<void> {
  const resp = await post<null>('/middleware/update', { id, config })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to update middleware')
}