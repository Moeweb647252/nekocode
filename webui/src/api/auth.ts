import { post } from './client'
import type { Token } from './types'

/** POST /api/auth — authenticate and receive a session token. */
export async function login(password: string): Promise<Token> {
  const resp = await post<Token>('/auth', {
    Password: { password },
  })
  if (resp.code !== 'ok') {
    throw new Error(resp.msg ?? 'Authentication failed')
  }
  return resp.data
}
