import { get, post } from './client'
import type { WorkspaceResponse } from './types'

// ── Workspace CRUD ──
// A workspace is unique per working directory; create is find-or-create.

/** POST /api/workspace/create (find-or-create by workingDirectory) */
export async function createWorkspace(
  workingDirectory: string,
  name?: string,
): Promise<WorkspaceResponse> {
  const resp = await post<WorkspaceResponse>('/workspace/create', {
    workingDirectory,
    name: name ?? null,
  })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to create workspace')
  return resp.data
}

/** GET /api/workspace/list — each workspace carries its threads */
export async function listWorkspaces(): Promise<WorkspaceResponse[]> {
  const resp = await get<WorkspaceResponse[]>('/workspace/list')
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to list workspaces')
  return resp.data
}

/** POST /api/workspace/get */
export async function getWorkspace(id: number): Promise<WorkspaceResponse> {
  const resp = await post<WorkspaceResponse>('/workspace/get', { id })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to get workspace')
  return resp.data
}

/** POST /api/workspace/update (rename) */
export async function updateWorkspace(id: number, name?: string): Promise<void> {
  const resp = await post<null>('/workspace/update', { id, name: name ?? null })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to update workspace')
}

/** POST /api/workspace/delete (cascades to all threads in the workspace) */
export async function deleteWorkspace(id: number): Promise<void> {
  const resp = await post<null>('/workspace/delete', { id })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to delete workspace')
}
