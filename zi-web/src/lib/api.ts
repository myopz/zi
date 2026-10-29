import { readAuth } from '@/lib/auth'
import type {
  DefaultsResponse,
  ModelsResponse,
  Session,
  SessionDetail,
  SessionStatus,
  User,
  Workspace,
} from '@/lib/types'

export class ApiError extends Error {
  status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

/** Turn HTML/SPA fallback bodies into a short, user-safe message. */
export function formatApiError(err: unknown): string {
  if (err instanceof ApiError) {
    return err.message
  }
  if (err instanceof Error) {
    return sanitizeErrorBody(err.message)
  }
  return sanitizeErrorBody(String(err))
}

function sanitizeErrorBody(raw: string): string {
  const trimmed = raw.trim()
  if (!trimmed) {
    return 'Request failed'
  }
  if (
    /<!doctype html/i.test(trimmed) ||
    /<html[\s>]/i.test(trimmed) ||
    trimmed.startsWith('<')
  ) {
    return 'API returned HTML instead of JSON — restart `zi --mode web` (new routes) or check the Vite `/api` proxy'
  }
  try {
    const parsed = JSON.parse(trimmed) as { error?: string }
    if (typeof parsed.error === 'string' && parsed.error.trim()) {
      return parsed.error.trim()
    }
  } catch {
    // not JSON
  }
  return trimmed.length > 180 ? `${trimmed.slice(0, 180)}…` : trimmed
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const auth = readAuth()
  const headers = new Headers(init.headers)
  if (!headers.has('Content-Type') && init.body) {
    headers.set('Content-Type', 'application/json')
  }
  if (auth?.accessToken) {
    headers.set('Authorization', `Bearer ${auth.accessToken}`)
  }
  const res = await fetch(path, { ...init, headers })
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new ApiError(
      res.status,
      sanitizeErrorBody(text || `${res.status} ${res.statusText}`),
    )
  }
  if (res.status === 204) {
    return undefined as T
  }
  const contentType = res.headers.get('content-type') ?? ''
  if (!contentType.includes('application/json')) {
    const text = await res.text().catch(() => '')
    throw new ApiError(
      res.status,
      sanitizeErrorBody(text || `Unexpected content-type: ${contentType}`),
    )
  }
  return (await res.json()) as T
}

export function getHealth(): Promise<{ ok: boolean }> {
  return request('/api/health')
}

export function getMe(): Promise<User> {
  return request('/api/me')
}

export function getDefaults(): Promise<DefaultsResponse> {
  return request('/api/defaults')
}

/** Prefer server default under ~/.zi; empty cwd lets a current server pick it. */
export async function resolveDefaultWorkspaceCwd(): Promise<string> {
  try {
    const defaults = await getDefaults()
    if (defaults.defaultWorkspaceCwd?.trim()) {
      return defaults.defaultWorkspaceCwd.trim()
    }
  } catch {
    // Old servers without /api/defaults — fall through.
  }
  return ''
}

export function listWorkspaces(): Promise<Workspace[]> {
  return request('/api/workspaces')
}

export function createWorkspace(input: {
  cwd: string
  name?: string
}): Promise<Workspace> {
  return request('/api/workspaces', {
    method: 'POST',
    body: JSON.stringify(input),
  })
}

/** Ensure Default cwd is under ~/.zi/agent/web/workspaces/<userId>. */
export function ensureDefaultWorkspace(): Promise<Workspace> {
  return request('/api/workspaces/ensure-default', {
    method: 'POST',
    body: JSON.stringify({}),
  })
}

export function deleteWorkspace(id: string): Promise<void> {
  return request(`/api/workspaces/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  })
}

export function renameWorkspace(
  id: string,
  name: string,
): Promise<Workspace> {
  return request(`/api/workspaces/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ name }),
  })
}

export function listSessions(workspaceId?: string): Promise<Session[]> {
  const qs = workspaceId
    ? `?workspaceId=${encodeURIComponent(workspaceId)}`
    : ''
  return request(`/api/sessions${qs}`)
}

export function createSession(input: {
  workspaceId: string
  title?: string
  model?: string
}): Promise<Session> {
  return request('/api/sessions', {
    method: 'POST',
    body: JSON.stringify(input),
  })
}

export function getSession(id: string): Promise<SessionDetail> {
  return request(`/api/sessions/${encodeURIComponent(id)}`)
}

export function deleteSession(id: string): Promise<void> {
  return request(`/api/sessions/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  })
}

export function renameSession(id: string, title: string): Promise<Session> {
  return request(`/api/sessions/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ title }),
  })
}

export function listModels(): Promise<ModelsResponse> {
  return request('/api/models')
}

export function setModel(input: {
  model: string
  sessionId?: string
}): Promise<{ model: string }> {
  return request('/api/model', {
    method: 'POST',
    body: JSON.stringify(input),
  })
}

export function getStatus(sessionId?: string): Promise<SessionStatus> {
  const qs = sessionId ? `?sessionId=${encodeURIComponent(sessionId)}` : ''
  return request(`/api/status${qs}`)
}
