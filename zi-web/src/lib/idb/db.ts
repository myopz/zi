import Dexie, { type EntityTable } from 'dexie'
import type { AgentMessage, Session, Workspace } from '@/lib/types'

export type CachedWorkspace = Workspace

export type CachedSession = Session

export type CachedTranscript = {
  sessionId: string
  revision: number
  messages: AgentMessage[]
  updatedAt: string
}

export type CachedDraft = {
  sessionId: string
  text: string
  updatedAt: string
}

export type CachedPrefs = {
  key: string
  value: unknown
}

export type CachedMeta = {
  key: string
  value: unknown
}

export type ZiUserDb = Dexie & {
  workspaces: EntityTable<CachedWorkspace, 'id'>
  sessions: EntityTable<CachedSession, 'id'>
  transcripts: EntityTable<CachedTranscript, 'sessionId'>
  drafts: EntityTable<CachedDraft, 'sessionId'>
  prefs: EntityTable<CachedPrefs, 'key'>
  meta: EntityTable<CachedMeta, 'key'>
}

const dbCache = new Map<string, ZiUserDb>()

export function getUserDb(userId: string): ZiUserDb {
  const existing = dbCache.get(userId)
  if (existing) {
    return existing
  }
  const db = new Dexie(`zi-web:${userId}`) as ZiUserDb
  db.version(1).stores({
    workspaces: 'id, updatedAt, cwd',
    sessions: 'id, workspaceId, updatedAt, revision',
    transcripts: 'sessionId, revision, updatedAt',
    drafts: 'sessionId, updatedAt',
    prefs: 'key',
    meta: 'key',
  })
  dbCache.set(userId, db)
  return db
}

export async function clearUserDb(userId: string): Promise<void> {
  const db = getUserDb(userId)
  await db.delete()
  dbCache.delete(userId)
}
