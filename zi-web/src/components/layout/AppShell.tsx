import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Composer } from '@/components/chat/Composer'
import { Transcript } from '@/components/chat/Transcript'
import { RightPanel } from '@/components/layout/RightPanel'
import { Sidebar } from '@/components/layout/Sidebar'
import {
  createSession,
  createWorkspace,
  deleteSession,
  deleteWorkspace,
  ensureDefaultWorkspace,
  formatApiError,
  getStatus,
  listModels,
  renameSession,
  renameWorkspace,
  resolveDefaultWorkspaceCwd,
  setModel,
} from '@/lib/api'
import type { AuthState } from '@/lib/auth'
import { clearAuth, logoutRemote } from '@/lib/auth'
import { clearUserDb } from '@/lib/idb/db'
import {
  ensureSession,
  ensureWorkspace,
  getDraft,
  getPref,
  hydrateSessions,
  hydrateTranscript,
  hydrateWorkspaces,
  putSessionCache,
  putWorkspaceCache,
  removeSessionCache,
  removeWorkspaceCache,
  saveDraft,
  scheduleTranscriptWrite,
  setPref,
} from '@/lib/idb/sync'
import {
  appendOptimisticUser,
  createTranscriptState,
  reduceAgentEvent,
  transcriptToMessages,
} from '@/lib/transcript/reducer'
import type {
  AgentEvent,
  ModelInfo,
  Session,
  SessionStatus,
  TranscriptState,
  Workspace,
} from '@/lib/types'
import { AgentSocket } from '@/lib/ws'

export function AppShell({
  auth,
  onLogout,
}: {
  auth: AuthState
  onLogout: () => void
}) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [workspaceId, setWorkspaceId] = useState<string | null>(null)
  const [sessions, setSessions] = useState<Session[]>([])
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [transcript, setTranscript] = useState<TranscriptState>(
    createTranscriptState(),
  )
  const [draft, setDraft] = useState('')
  const [models, setModels] = useState<ModelInfo[]>([])
  const [model, setModelId] = useState('')
  const [status, setStatus] = useState<SessionStatus | null>(null)
  const [connected, setConnected] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [bootError, setBootError] = useState<string | null>(null)
  const [suggestedWorkspaceCwd, setSuggestedWorkspaceCwd] = useState('')
  const socketRef = useRef<AgentSocket | null>(null)
  const transcriptRef = useRef(transcript)
  transcriptRef.current = transcript
  const sessionIdRef = useRef(sessionId)
  sessionIdRef.current = sessionId
  const userIdRef = useRef(auth.user.id)
  userIdRef.current = auth.user.id
  const transcriptLoadGen = useRef(0)

  const activeSession = useMemo(
    () => sessions.find((s) => s.id === sessionId) ?? null,
    [sessions, sessionId],
  )

  const persistTranscript = useCallback((next: TranscriptState) => {
    const sid = sessionIdRef.current
    if (!sid) {
      return
    }
    scheduleTranscriptWrite(
      userIdRef.current,
      sid,
      next.revision,
      transcriptToMessages(next),
    )
  }, [])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const [prefsWorkspace, prefsSession, modelRes] = await Promise.all([
          getPref<string | null>(auth.user.id, 'workspaceId', null),
          getPref<string | null>(auth.user.id, 'sessionId', null),
          listModels().catch(() => ({
            models: [] as ModelInfo[],
            current: undefined,
          })),
        ])
        if (cancelled) {
          return
        }
        // Always pin Default to ~/.zi/agent/web/workspaces/<userId> (migrates
        // legacy rows that pointed at a git checkout).
        const suggestedCwd = await resolveDefaultWorkspaceCwd()
        if (!cancelled) {
          setSuggestedWorkspaceCwd(suggestedCwd)
        }
        let defaultWs: Workspace | null = null
        try {
          defaultWs = await ensureDefaultWorkspace()
        } catch {
          defaultWs = await ensureWorkspace(
            auth.user.id,
            suggestedCwd,
            'Default',
          )
        }
        const nextWorkspaces = await hydrateWorkspaces(auth.user.id)
        setWorkspaces(nextWorkspaces)
        setModels(modelRes.models)
        const initialWorkspace =
          nextWorkspaces.find((w) => w.id === defaultWs?.id)?.id ??
          nextWorkspaces.find((w) => w.name === 'Default')?.id ??
          nextWorkspaces.find((w) => w.id === prefsWorkspace)?.id ??
          nextWorkspaces[0]?.id ??
          null
        setWorkspaceId(initialWorkspace)
        if (modelRes.current) {
          setModelId(modelRes.current)
        }
        if (initialWorkspace) {
          const ensured = await ensureSession(
            auth.user.id,
            initialWorkspace,
            modelRes.current,
          )
          if (cancelled) {
            return
          }
          const sess = await hydrateSessions(auth.user.id, initialWorkspace)
          const list = sess.some((s) => s.id === ensured.id)
            ? sess
            : [ensured, ...sess]
          setSessions(list)
          const initialSession =
            list.find((s) => s.id === prefsSession)?.id ??
            ensured.id ??
            list[0]?.id ??
            null
          setSessionId(initialSession)
          if (!modelRes.current && list[0]?.model) {
            setModelId(list[0].model)
          }
        }
      } catch (err) {
        if (!cancelled) {
          setBootError(formatApiError(err))
        }
      }
    })()
    return () => {
      cancelled = true
    }
  }, [auth.user.id])

  useEffect(() => {
    if (!workspaceId) {
      setSessions([])
      setSessionId(null)
      return
    }
    void setPref(auth.user.id, 'workspaceId', workspaceId)
    let cancelled = false
    void (async () => {
      const sess = await hydrateSessions(auth.user.id, workspaceId)
      if (cancelled) {
        return
      }
      setSessions(sess)
      setSessionId((prev) =>
        sess.some((s) => s.id === prev) ? prev : (sess[0]?.id ?? null),
      )
    })()
    return () => {
      cancelled = true
    }
  }, [auth.user.id, workspaceId])

  useEffect(() => {
    if (!sessionId) {
      setTranscript(createTranscriptState())
      setDraft('')
      return
    }
    void setPref(auth.user.id, 'sessionId', sessionId)
    const loadGen = ++transcriptLoadGen.current
    let cancelled = false
    void (async () => {
      const [loaded, draftText, sessionStatus] = await Promise.all([
        hydrateTranscript(auth.user.id, sessionId),
        getDraft(auth.user.id, sessionId),
        getStatus(sessionId).catch(() => null),
      ])
      if (cancelled || loadGen !== transcriptLoadGen.current) {
        return
      }
      // Never clobber a live or newer local transcript (first greeting replies
      // often finish before IndexedDB hydrate returns a stale empty/partial cache).
      setTranscript((prev) => {
        if (loadGen !== transcriptLoadGen.current) {
          return prev
        }
        if (
          prev.busy ||
          prev.items.some(
            (item) =>
              (item.kind === 'assistant' && item.streaming) ||
              item.kind === 'assistant' ||
              item.kind === 'tool',
          )
        ) {
          return prev
        }
        const loadedCount = loaded.messages.length
        if (prev.items.length > loadedCount) {
          return prev
        }
        return createTranscriptState(loaded.messages, loaded.revision)
      })
      setDraft(draftText)
      if (sessionStatus) {
        setStatus(sessionStatus)
        if (sessionStatus.model) {
          setModelId(sessionStatus.model)
        }
      }
      const session = sessions.find((s) => s.id === sessionId)
      if (session?.model) {
        setModelId((prev) => prev || session.model)
      }
    })()
    return () => {
      cancelled = true
    }
    // sessions intentionally omitted: avoid reload loops when list refreshes
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth.user.id, sessionId])

  // One WS per login — sessionId travels on each prompt; do not reconnect on switch.
  useEffect(() => {
    const socket = new AgentSocket(auth.accessToken, {
      onConnectionChange: setConnected,
      onEvent: (event: AgentEvent) => {
        setTranscript((prev) => {
          const next = reduceAgentEvent(prev, event)
          persistTranscript(next)
          return next
        })
        if (event.type === 'agent_end' || event.type === 'agent_start') {
          const sid = sessionIdRef.current
          if (sid) {
            void getStatus(sid)
              .then(setStatus)
              .catch(() => undefined)
          }
        }
      },
      onSessionMeta: (meta) => {
        setTranscript((prev) => {
          const next = { ...prev, revision: meta.revision }
          persistTranscript(next)
          return next
        })
        setSessions((prev) =>
          prev.map((s) =>
            s.id === meta.sessionId
              ? {
                  ...s,
                  revision: meta.revision,
                  title: meta.title ?? s.title,
                  model: meta.model ?? s.model,
                  updatedAt: new Date().toISOString(),
                }
              : s,
          ),
        )
        if (meta.model) {
          setModelId(meta.model)
        }
      },
      onResponse: (msg) => {
        if (!msg.success && msg.command === 'prompt') {
          setBootError(msg.error || 'Prompt failed')
          setTranscript((prev) => ({ ...prev, busy: false }))
        }
      },
      onError: (message) => {
        setBootError(message)
      },
    })
    socketRef.current = socket
    return () => {
      socket.close()
      socketRef.current = null
    }
  }, [auth.accessToken, persistTranscript])

  async function handleLogout() {
    await logoutRemote(auth.accessToken)
    await clearUserDb(auth.user.id)
    clearAuth()
    onLogout()
  }

  async function handleNewSession() {
    try {
      let wsId = workspaceId
      if (!wsId) {
        const defaultCwd = await resolveDefaultWorkspaceCwd()
        const ws = await ensureWorkspace(
          auth.user.id,
          defaultCwd,
          'Default',
        )
        await putWorkspaceCache(auth.user.id, ws)
        setWorkspaces((prev) =>
          prev.some((w) => w.id === ws.id) ? prev : [ws, ...prev],
        )
        setWorkspaceId(ws.id)
        wsId = ws.id
      }
      const created = await createSession({
        workspaceId: wsId,
        title: 'New session',
        model: model || undefined,
      })
      await putSessionCache(auth.user.id, created)
      setSessions((prev) => [created, ...prev])
      setSessionId(created.id)
      setTranscript(createTranscriptState([], 0))
    } catch (err) {
      setBootError(formatApiError(err))
    }
  }

  async function handleAddWorkspace(cwd: string, name?: string) {
    const trimmed = cwd.trim()
    if (!trimmed) {
      throw new Error('Workspace path is required')
    }
    // cwd is unique per user — prefilled Default path would silently no-op.
    if (workspaces.some((w) => w.cwd === trimmed)) {
      throw new Error(
        'A workspace with this path already exists — pick a different cwd',
      )
    }
    const created = await createWorkspace({
      cwd: trimmed,
      name,
    })
    if (workspaces.some((w) => w.id === created.id)) {
      throw new Error(
        'A workspace with this path already exists — pick a different cwd',
      )
    }
    await putWorkspaceCache(auth.user.id, created)
    setWorkspaces((prev) => [created, ...prev.filter((w) => w.id !== created.id)])
    setWorkspaceId(created.id)
    const list = await hydrateSessions(auth.user.id, created.id)
    if (list.length === 0) {
      const session = await ensureSession(
        auth.user.id,
        created.id,
        model || undefined,
      )
      setSessions([session])
      setSessionId(session.id)
      setTranscript(createTranscriptState([], 0))
    } else {
      setSessions(list)
      setSessionId(list[0]?.id ?? null)
    }
  }

  async function handleRenameWorkspace(id: string, name: string) {
    const updated = await renameWorkspace(id, name)
    await putWorkspaceCache(auth.user.id, updated)
    setWorkspaces((prev) =>
      prev.map((w) => (w.id === id ? updated : w)),
    )
  }

  async function handleRenameSession(id: string, title: string) {
    const updated = await renameSession(id, title)
    await putSessionCache(auth.user.id, updated)
    setSessions((prev) => prev.map((s) => (s.id === id ? updated : s)))
  }

  async function handleDeleteWorkspace(id: string) {
    const target = workspaces.find((w) => w.id === id)
    const label = target ? `${target.name} (${target.cwd})` : 'this workspace'
    if (
      !window.confirm(
        `Delete workspace ${label}? All sessions under it will be removed.`,
      )
    ) {
      return
    }
    try {
      await deleteWorkspace(id)
      await removeWorkspaceCache(auth.user.id, id)
      const next = workspaces.filter((w) => w.id !== id)
      setWorkspaces(next)
      if (workspaceId === id) {
        const fallback = next[0]?.id ?? null
        setWorkspaceId(fallback)
        if (!fallback) {
          setSessions([])
          setSessionId(null)
          setTranscript(createTranscriptState())
          setDraft('')
        }
      }
    } catch (err) {
      setBootError(formatApiError(err))
    }
  }

  async function handleDeleteSession(id: string) {
    const target = sessions.find((s) => s.id === id)
    const label = target?.title || 'this session'
    if (!window.confirm(`Delete ${label}?`)) {
      return
    }
    try {
      await deleteSession(id)
      await removeSessionCache(auth.user.id, id)
      const next = sessions.filter((s) => s.id !== id)
      setSessions(next)
      if (sessionId === id) {
        const fallback = next[0]?.id ?? null
        setSessionId(fallback)
        if (!fallback) {
          setTranscript(createTranscriptState())
          setDraft('')
        }
      }
    } catch (err) {
      setBootError(formatApiError(err))
    }
  }

  function handleDraftChange(value: string) {
    setDraft(value)
    if (sessionId) {
      void saveDraft(auth.user.id, sessionId, value)
    }
  }

  function handleSubmit(text: string) {
    if (!sessionId || !socketRef.current) {
      return
    }
    // Invalidate in-flight session hydrates so they cannot wipe this turn.
    transcriptLoadGen.current += 1
    setTranscript((prev) => {
      const next = appendOptimisticUser(prev, text)
      persistTranscript(next)
      return next
    })
    setDraft('')
    void saveDraft(auth.user.id, sessionId, '')
    socketRef.current.prompt(sessionId, text)
  }

  function handleAbort() {
    if (!sessionId || !socketRef.current) {
      return
    }
    socketRef.current.abort(sessionId)
  }

  async function handleModelChange(nextModel: string) {
    setModelId(nextModel)
    try {
      await setModel({ model: nextModel, sessionId: sessionId ?? undefined })
      if (sessionId) {
        setSessions((prev) =>
          prev.map((s) =>
            s.id === sessionId ? { ...s, model: nextModel } : s,
          ),
        )
      }
    } catch (err) {
      setBootError(formatApiError(err))
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <Sidebar
        user={auth.user}
        workspaces={workspaces}
        workspaceId={workspaceId}
        sessions={sessions}
        sessionId={sessionId}
        suggestedWorkspaceCwd={suggestedWorkspaceCwd}
        onSelectWorkspace={setWorkspaceId}
        onSelectSession={setSessionId}
        onDeleteSession={(id) => {
          void handleDeleteSession(id)
        }}
        onDeleteWorkspace={(id) => {
          void handleDeleteWorkspace(id)
        }}
        onRenameWorkspace={handleRenameWorkspace}
        onRenameSession={handleRenameSession}
        onNewSession={() => {
          void handleNewSession()
        }}
        onAddWorkspace={handleAddWorkspace}
        onOpenSettings={() => setSettingsOpen(true)}
        onLogout={() => {
          void handleLogout()
        }}
      />
      <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
          <div className="min-w-0 truncate text-sm font-medium">
            {activeSession?.title ?? 'No session'}
          </div>
          {bootError ? (
            <button
              type="button"
              className="ml-auto min-w-0 max-w-[50%] shrink truncate text-xs text-destructive hover:underline"
              onClick={() => setBootError(null)}
              title={bootError}
            >
              {bootError}
            </button>
          ) : (
            <div className="ml-auto text-xs text-muted-foreground">
              {transcript.items.length} items · cached via IndexedDB
            </div>
          )}
        </header>
        <div className="min-h-0 flex-1">
          <Transcript items={transcript.items} />
        </div>
        <Composer
          draft={draft}
          onDraftChange={handleDraftChange}
          models={models}
          model={model || activeSession?.model || ''}
          onModelChange={(m) => {
            void handleModelChange(m)
          }}
          busy={transcript.busy || Boolean(status?.busy)}
          connected={connected}
          disabled={!sessionId}
          onSubmit={handleSubmit}
          onAbort={handleAbort}
        />
      </main>
      <RightPanel
        status={status}
        revision={transcript.revision}
        settingsOpen={settingsOpen}
        username={auth.user.username}
        onCloseSettings={() => setSettingsOpen(false)}
      />
    </div>
  )
}
