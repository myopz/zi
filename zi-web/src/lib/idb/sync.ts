import {
  createSession,
  createWorkspace,
  getSession,
  listSessions,
  listWorkspaces,
} from '@/lib/api'
import { getUserDb } from '@/lib/idb/db'
import type { AgentMessage, Session, Workspace } from '@/lib/types'

const TRANSCRIPT_FLUSH_MS = 400

const pendingTranscripts = new Map<
  string,
  {
    userId: string
    sessionId: string
    revision: number
    messages: AgentMessage[]
    timer: ReturnType<typeof setTimeout> | null
  }
>()

export async function hydrateWorkspaces(userId: string): Promise<Workspace[]> {
  const db = getUserDb(userId)
  const cached = await db.workspaces.orderBy('updatedAt').reverse().toArray()
  try {
    const remote = await listWorkspaces()
    await db.workspaces.clear()
    if (remote.length > 0) {
      await db.workspaces.bulkPut(remote)
    }
    return remote
  } catch {
    return cached
  }
}

export async function hydrateSessions(
  userId: string,
  workspaceId?: string,
): Promise<Session[]> {
  const db = getUserDb(userId)
  const cached = workspaceId
    ? await db.sessions.where('workspaceId').equals(workspaceId).toArray()
    : await db.sessions.toArray()
  cached.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
  try {
    const remote = await listSessions(workspaceId)
    if (workspaceId) {
      const others = await db.sessions
        .filter((s) => s.workspaceId !== workspaceId)
        .toArray()
      await db.sessions.clear()
      await db.sessions.bulkPut([...others, ...remote])
    } else {
      await db.sessions.clear()
      if (remote.length > 0) {
        await db.sessions.bulkPut(remote)
      }
    }
    return remote
  } catch {
    return cached
  }
}

export async function hydrateTranscript(
  userId: string,
  sessionId: string,
): Promise<{ messages: AgentMessage[]; revision: number; fromCache: boolean }> {
  const db = getUserDb(userId)
  const cached = await db.transcripts.get(sessionId)
  try {
    const detail = await getSession(sessionId)
    const remoteRevision = detail.revision
    if (cached && cached.revision >= remoteRevision) {
      await db.sessions.put(detail.session)
      return {
        messages: cached.messages,
        revision: cached.revision,
        fromCache: true,
      }
    }
    await db.sessions.put(detail.session)
    await db.transcripts.put({
      sessionId,
      revision: remoteRevision,
      messages: detail.messages,
      updatedAt: new Date().toISOString(),
    })
    return {
      messages: detail.messages,
      revision: remoteRevision,
      fromCache: false,
    }
  } catch {
    return {
      messages: cached?.messages ?? [],
      revision: cached?.revision ?? 0,
      fromCache: true,
    }
  }
}

export function scheduleTranscriptWrite(
  userId: string,
  sessionId: string,
  revision: number,
  messages: AgentMessage[],
): void {
  const key = `${userId}:${sessionId}`
  const existing = pendingTranscripts.get(key)
  if (existing?.timer) {
    clearTimeout(existing.timer)
  }
  const entry = {
    userId,
    sessionId,
    revision,
    messages,
    timer: null as ReturnType<typeof setTimeout> | null,
  }
  entry.timer = setTimeout(() => {
    void flushTranscript(key)
  }, TRANSCRIPT_FLUSH_MS)
  pendingTranscripts.set(key, entry)
}

async function flushTranscript(key: string): Promise<void> {
  const entry = pendingTranscripts.get(key)
  if (!entry) {
    return
  }
  pendingTranscripts.delete(key)
  const db = getUserDb(entry.userId)
  const current = await db.transcripts.get(entry.sessionId)
  if (current && current.revision > entry.revision) {
    return
  }
  await db.transcripts.put({
    sessionId: entry.sessionId,
    revision: entry.revision,
    messages: entry.messages,
    updatedAt: new Date().toISOString(),
  })
}

export async function putSessionCache(
  userId: string,
  session: Session,
): Promise<void> {
  await getUserDb(userId).sessions.put(session)
}

export async function removeSessionCache(
  userId: string,
  sessionId: string,
): Promise<void> {
  const db = getUserDb(userId)
  await db.transaction(
    'rw',
    db.sessions,
    db.transcripts,
    db.drafts,
    async () => {
      await db.sessions.delete(sessionId)
      await db.transcripts.delete(sessionId)
      await db.drafts.delete(sessionId)
    },
  )
}

export async function putWorkspaceCache(
  userId: string,
  workspace: Workspace,
): Promise<void> {
  await getUserDb(userId).workspaces.put(workspace)
}

export async function removeWorkspaceCache(
  userId: string,
  workspaceId: string,
): Promise<void> {
  const db = getUserDb(userId)
  const sessionIds = (
    await db.sessions.where('workspaceId').equals(workspaceId).toArray()
  ).map((s) => s.id)
  await db.transaction(
    'rw',
    db.workspaces,
    db.sessions,
    db.transcripts,
    db.drafts,
    async () => {
      await db.workspaces.delete(workspaceId)
      for (const sid of sessionIds) {
        await db.sessions.delete(sid)
        await db.transcripts.delete(sid)
        await db.drafts.delete(sid)
      }
    },
  )
}

export async function getDraft(
  userId: string,
  sessionId: string,
): Promise<string> {
  const row = await getUserDb(userId).drafts.get(sessionId)
  return row?.text ?? ''
}

export async function saveDraft(
  userId: string,
  sessionId: string,
  text: string,
): Promise<void> {
  await getUserDb(userId).drafts.put({
    sessionId,
    text,
    updatedAt: new Date().toISOString(),
  })
}

export async function getPref<T>(
  userId: string,
  key: string,
  fallback: T,
): Promise<T> {
  const row = await getUserDb(userId).prefs.get(key)
  return (row?.value as T | undefined) ?? fallback
}

export async function setPref(
  userId: string,
  key: string,
  value: unknown,
): Promise<void> {
  await getUserDb(userId).prefs.put({ key, value })
}

export async function ensureWorkspace(
  userId: string,
  cwd: string,
  name?: string,
): Promise<Workspace> {
  const workspaces = await hydrateWorkspaces(userId)
  const trimmed = cwd.trim()
  // Empty cwd → server chooses `$ZI_AGENT_DIR/web/workspaces/<userId>`.
  if (trimmed) {
    const found = workspaces.find((w) => w.cwd === trimmed)
    if (found) {
      return found
    }
  } else if (workspaces.length > 0) {
    const byName = workspaces.find((w) => w.name === (name ?? 'Default'))
    return byName ?? workspaces[0]!
  }
  const created = await createWorkspace({
    cwd: trimmed,
    name,
  })
  await putWorkspaceCache(userId, created)
  return created
}

export async function ensureSession(
  userId: string,
  workspaceId: string,
  model?: string,
): Promise<Session> {
  const sessions = await hydrateSessions(userId, workspaceId)
  if (sessions[0]) {
    return sessions[0]
  }
  const created = await createSession({
    workspaceId,
    title: 'New session',
    model,
  })
  await putSessionCache(userId, created)
  return created
}
