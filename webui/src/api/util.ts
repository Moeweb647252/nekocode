import { get, post } from './client'

/** Entry returned by POST /api/util/fs/list_dir */
export interface ListDirEntry {
  name: string
  isDir: boolean
  metadata: {
    size: number
    createdAt: number | null
    modifiedAt: number | null
  }
}

/** POST /api/util/fs/list_dir — list contents of a directory on the server. */
export async function listDir(path: string): Promise<ListDirEntry[]> {
  const resp = await post<ListDirEntry[]>('/util/fs/list_dir', { path })
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to list directory')
  return resp.data
}

/** Response from GET /api/util/fs/dirs */
export interface DirsResponse {
  homeDir: string
}

/** GET /api/util/fs/dirs — get known directories on the server. */
export async function getDirs(): Promise<DirsResponse> {
  const resp = await get<DirsResponse>('/util/fs/dirs')
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to get directories')
  return resp.data
}

/** GET /api/util/models — available model names from the server config. */
export async function getModels(): Promise<string[]> {
  const resp = await get<string[]>('/util/models')
  if (resp.code !== 'ok') throw new Error(resp.msg ?? 'Failed to get models')
  return resp.data
}
