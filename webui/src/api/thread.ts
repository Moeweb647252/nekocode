import { get, post } from './client'
import type { GetThreadResponse, Thread } from './types'

// ── Thread CRUD ──

/** POST /api/thread/create */
export async function createThread(workingDirectory: string): Promise<Thread> {
  const resp = await post<Thread>('/thread/create', { workingDirectory })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to create thread')
  return resp.data
}

/** GET /api/thread/list */
export async function listThreads(): Promise<Thread[]> {
  const resp = await get<Thread[]>('/thread/list')
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to list threads')
  return resp.data
}

/** POST /api/thread/delete */
export async function deleteThread(id: number): Promise<void> {
  const resp = await post<null>('/thread/delete', { id })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to delete thread')
}

/** POST /api/thread/activate */
export async function activateThread(id: number): Promise<void> {
  const resp = await post<null>('/thread/activate', { id })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to activate thread')
}

/** POST /api/thread/get */
export async function getThread(
  id: number,
  turnsLimit?: number,
): Promise<GetThreadResponse> {
  const resp = await post<GetThreadResponse>('/thread/get', {
    id,
    turnsLimit: turnsLimit ?? null,
  })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to get thread')
  return resp.data
}

/** POST /api/thread/update */
export async function updateThread(
  id: number,
  title?: string | null,
  model?: string | null,
): Promise<void> {
  const resp = await post<null>('/thread/update', {
    id,
    title: title ?? null,
    model: model ?? null,
  })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to update thread')
}
