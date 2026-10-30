import { PanelLeft, PanelRight } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Composer } from '@/components/chat/Composer'
import { Transcript } from '@/components/chat/Transcript'
import { RightPanel } from '@/components/layout/RightPanel'
import { Sidebar } from '@/components/layout/Sidebar'
import { Button } from '@/components/ui/button'
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
  applySlashResult,
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
import { formatTokens, sumTranscriptUsage } from '@/lib/usage'
import { useMediaQuery } from '@/lib/use-media-query'
import { AgentSocket } from '@/lib/ws'

const LEFT_OPEN_PREF = 'layout.leftOpen'
const RIGHT_OPEN_PREF = 'layout.rightOpen'

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
  const isDesktop = useMediaQuery('(min-width: 768px)')
  // Desktop: in-flow panels, remembered. Mobile: overlay drawers, start closed.
  const [leftOpen, setLeftOpen] = useState(true)
  const [rightOpen, setRightOpen] = useState(true)
  const [mobileLeftOpen, setMobileLeftOpen] = useState(false)
  const [mobileRightOpen, setMobileRightOpen] = useState(false)
  const socketRef = useRef<AgentSocket | null>(null)
  const transcriptRef = useRef(transcript)
  transcriptRef.current = transcript
  const sessionIdRef = useRef(sessionId)
  sessionIdRef.current = sessionId
  const userIdRef = useRef(auth.user.id)
  userIdRef.current = auth.user.id
  const transcriptLoadGen = useRef(0)
  const layoutPrefsReady = useRef(false)

  const activeSession = useMemo(
    () => sessions.find((s) => s.id === sessionId) ?? null,
    [sessions, sessionId],
  )

  const sessionTokenTotals = useMemo(
    () => sumTranscriptUsage(transcript.items),
    [transcript.items],
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
      const [storedLeft, storedRight] = await Promise.all([
        getPref<boolean>(auth.user.id, LEFT_OPEN_PREF, true),
        getPref<boolean>(auth.user.id, RIGHT_OPEN_PREF, true),
      ])
      if (cancelled) {
        return
      }
      setLeftOpen(storedLeft)
      setRightOpen(storedRight)
      layoutPrefsReady.current = true
    })()
    return () => {
      cancelled = true
    }
  }, [auth.user.id])

  useEffect(() => {
    if (!layoutPrefsReady.current) {
      return
    }
    void setPref(auth.user.id, LEFT_OPEN_PREF, leftOpen)
  }, [auth.user.id, leftOpen])

  useEffect(() => {
    if (!layoutPrefsReady.current) {
      return
    }
    void setPref(auth.user.id, RIGHT_OPEN_PREF, rightOpen)
  }, [auth.user.id, rightOpen])

  useEffect(() => {
    if (isDesktop) {
      setMobileLeftOpen(false)
      setMobileRightOpen(false)
    }
  }, [isDesktop])

  useEffect(() => {
    if (!mobileLeftOpen && !mobileRightOpen) {
      return
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setMobileLeftOpen(false)
        setMobileRightOpen(false)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [mobileLeftOpen, mobileRightOpen])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const [prefsWorkspace, modelRes] = await Promise.all([
          getPref<string | null>(auth.user.id, 'workspaceId', null),
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
        if (cancelled) {
          return
        }
        setWorkspaces(nextWorkspaces)
        setModels(modelRes.models)
        if (modelRes.current) {
          setModelId(modelRes.current)
        }
        // Session list/selection is owned by the workspaceId effect below.
        const initialWorkspace =
          nextWorkspaces.find((w) => w.id === prefsWorkspace)?.id ??
          nextWorkspaces.find((w) => w.id === defaultWs?.id)?.id ??
          nextWorkspaces.find((w) => w.name === 'Default')?.id ??
          nextWorkspaces[0]?.id ??
          null
        setWorkspaceId(initialWorkspace)
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
      setTranscript(createTranscriptState())
      setDraft('')
      setStatus(null)
      return
    }
    void setPref(auth.user.id, 'workspaceId', workspaceId)
    let cancelled = false
    void (async () => {
      // Drop the previous workspace's selection immediately so the center pane
      // cannot keep showing another workspace's session while we hydrate.
      setSessionId(null)
      setTranscript(createTranscriptState())
      setDraft('')
      setStatus(null)

      let sess = await hydrateSessions(auth.user.id, workspaceId)
      if (cancelled) {
        return
      }
      if (sess.length === 0) {
        const created = await ensureSession(
          auth.user.id,
          workspaceId,
          model || undefined,
        )
        if (cancelled) {
          return
        }
        sess = [created]
      }
      const [rememberedForWorkspace, rememberedGlobal] = await Promise.all([
        getPref<string | null>(
          auth.user.id,
          `sessionId:${workspaceId}`,
          null,
        ),
        getPref<string | null>(auth.user.id, 'sessionId', null),
      ])
      if (cancelled) {
        return
      }
      const remembered = rememberedForWorkspace ?? rememberedGlobal
      const nextId =
        sess.find((s) => s.id === remembered)?.id ?? sess[0]?.id ?? null
      setSessions(sess)
      setSessionId(nextId)
    })()
    return () => {
      cancelled = true
    }
    // `model` only seeds a brand-new empty workspace session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth.user.id, workspaceId])

  useEffect(() => {
    if (!sessionId) {
      setTranscript(createTranscriptState())
      setDraft('')
      setStatus(null)
      return
    }
    void setPref(auth.user.id, 'sessionId', sessionId)
    if (workspaceId) {
      void setPref(auth.user.id, `sessionId:${workspaceId}`, sessionId)
    }
    const loadGen = ++transcriptLoadGen.current
    // Clear previous session's transcript immediately on switch; hydrate fills in.
    setTranscript(createTranscriptState())
    setDraft('')
    setStatus(null)
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
      // Never clobber a *live* turn for this same session (prompt can finish
      // before IndexedDB/network hydrate returns a stale empty/partial cache).
      // Completed assistant/tool items from another session must not block switch.
      setTranscript((prev) => {
        if (loadGen !== transcriptLoadGen.current) {
          return prev
        }
        if (
          prev.busy ||
          prev.items.some(
            (item) => item.kind === 'assistant' && item.streaming,
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
    // sessions/workspaceId omitted: sessionId changes drive reload; workspace
    // effect clears selection before assigning the next workspace's session.
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
        const active = meta.sessionId === sessionIdRef.current
        if (active) {
          setTranscript((prev) => {
            const next = { ...prev, revision: meta.revision }
            persistTranscript(next)
            return next
          })
          if (meta.model) {
            setModelId(meta.model)
          }
        }
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
      },
      onSlashResult: (msg) => {
        if (msg.sessionId !== sessionIdRef.current) {
          return
        }
        setTranscript((prev) => {
          const next = applySlashResult(prev, msg)
          persistTranscript(next)
          return next
        })
        if (msg.model) {
          setModelId(msg.model)
          setSessions((prev) =>
            prev.map((s) =>
              s.id === msg.sessionId ? { ...s, model: msg.model! } : s,
            ),
          )
        }
        if (msg.kind === 'reset' && typeof msg.revision === 'number') {
          setSessions((prev) =>
            prev.map((s) =>
              s.id === msg.sessionId
                ? { ...s, revision: msg.revision! }
                : s,
            ),
          )
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
    // Workspace effect hydrates sessions and selects/creates one.
    setWorkspaceId(created.id)
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

  function openSettings() {
    setSettingsOpen(true)
    if (isDesktop) {
      setRightOpen(true)
    } else {
      setMobileLeftOpen(false)
      setMobileRightOpen(true)
    }
  }

  function toggleLeft() {
    if (isDesktop) {
      setLeftOpen((v) => !v)
    } else {
      setMobileRightOpen(false)
      setMobileLeftOpen((v) => !v)
    }
  }

  function toggleRight() {
    if (isDesktop) {
      setRightOpen((v) => !v)
    } else {
      setMobileLeftOpen(false)
      setMobileRightOpen((v) => !v)
    }
  }

  const showLeft = isDesktop ? leftOpen : mobileLeftOpen
  const showRight = isDesktop ? rightOpen : mobileRightOpen
  const mobileDrawerOpen = !isDesktop && (mobileLeftOpen || mobileRightOpen)

  return (
    <div className="relative flex h-full min-h-0 overflow-hidden">
      {mobileDrawerOpen ? (
        <button
          type="button"
          className="fixed inset-0 z-30 bg-black/40"
          aria-label="Close panel"
          onClick={() => {
            setMobileLeftOpen(false)
            setMobileRightOpen(false)
          }}
        />
      ) : null}

      {showLeft ? (
        <Sidebar
          user={auth.user}
          workspaces={workspaces}
          workspaceId={workspaceId}
          sessions={sessions}
          sessionId={sessionId}
          suggestedWorkspaceCwd={suggestedWorkspaceCwd}
          mobile={!isDesktop}
          onCollapse={() => {
            if (isDesktop) {
              setLeftOpen(false)
            } else {
              setMobileLeftOpen(false)
            }
          }}
          onSelectWorkspace={(id) => {
            if (id === workspaceId) {
              return
            }
            // Clear before workspaceId updates so the center pane cannot keep
            // rendering the previous workspace's session for a frame.
            setSessionId(null)
            setSessions([])
            setTranscript(createTranscriptState())
            setDraft('')
            setStatus(null)
            setWorkspaceId(id)
          }}
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
          onOpenSettings={openSettings}
          onLogout={() => {
            void handleLogout()
          }}
        />
      ) : null}

      <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-2 sm:gap-3 sm:px-4">
          {!showLeft || !isDesktop ? (
            <Button
              type="button"
              size="icon"
              variant="ghost"
              className="h-8 w-8 shrink-0"
              onClick={toggleLeft}
              title="Open sidebar"
              aria-label="Open sidebar"
              aria-pressed={showLeft}
            >
              <PanelLeft className="h-4 w-4" />
            </Button>
          ) : null}
          <div className="min-w-0 flex-1 truncate text-sm font-medium">
            {activeSession?.title ?? 'No session'}
          </div>
          {bootError ? (
            <button
              type="button"
              className="min-w-0 max-w-[40%] shrink truncate text-xs text-destructive hover:underline sm:max-w-[50%]"
              onClick={() => setBootError(null)}
              title={bootError}
            >
              {bootError}
            </button>
          ) : (
            <div className="hidden shrink-0 text-xs text-muted-foreground sm:block">
              {transcript.items.length} items
              {sessionTokenTotals.turns > 0
                ? ` · Σ in ${formatTokens(sessionTokenTotals.input)}`
                : ''}
              {' · '}
              IndexedDB
            </div>
          )}
          {!showRight || !isDesktop ? (
            <Button
              type="button"
              size="icon"
              variant="ghost"
              className="h-8 w-8 shrink-0"
              onClick={toggleRight}
              title="Open context panel"
              aria-label="Open context panel"
              aria-pressed={showRight}
            >
              <PanelRight className="h-4 w-4" />
            </Button>
          ) : null}
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
          workspaceId={workspaceId}
          contextUsage={transcript.contextUsage}
          sessionTokens={{
            input: sessionTokenTotals.input,
            output: sessionTokenTotals.output,
            turns: sessionTokenTotals.turns,
          }}
          onSubmit={handleSubmit}
          onAbort={handleAbort}
        />
      </main>

      {showRight ? (
        <RightPanel
          status={status}
          revision={transcript.revision}
          settingsOpen={settingsOpen}
          username={auth.user.username}
          mobile={!isDesktop}
          onCollapse={() => {
            if (isDesktop) {
              setRightOpen(false)
            } else {
              setMobileRightOpen(false)
            }
          }}
          onCloseSettings={() => setSettingsOpen(false)}
        />
      ) : null}
    </div>
  )
}
