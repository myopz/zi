import {
  FolderOpen,
  LogOut,
  MessageSquarePlus,
  Pencil,
  Plus,
  Settings,
  Trash2,
  X,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { HoverTip } from '@/components/ui/hover-tip'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { formatApiError } from '@/lib/api'
import type { Session, User, Workspace } from '@/lib/types'
import {
  cn,
  shortenPath,
  suggestNewWorkspaceCwd,
  workspaceLabel,
} from '@/lib/utils'

type RenameTarget =
  | { kind: 'workspace'; id: string }
  | { kind: 'session'; id: string }
  | null

export function Sidebar({
  user,
  workspaces,
  workspaceId,
  sessions,
  sessionId,
  suggestedWorkspaceCwd,
  onSelectWorkspace,
  onSelectSession,
  onDeleteSession,
  onDeleteWorkspace,
  onRenameWorkspace,
  onRenameSession,
  onNewSession,
  onAddWorkspace,
  onOpenSettings,
  onLogout,
}: {
  user: User
  workspaces: Workspace[]
  workspaceId: string | null
  sessions: Session[]
  sessionId: string | null
  suggestedWorkspaceCwd: string
  onSelectWorkspace: (id: string) => void
  onSelectSession: (id: string) => void
  onDeleteSession: (id: string) => void
  onDeleteWorkspace: (id: string) => void
  onRenameWorkspace: (id: string, name: string) => Promise<void>
  onRenameSession: (id: string, title: string) => Promise<void>
  onNewSession: () => void
  onAddWorkspace: (cwd: string, name?: string) => Promise<void> | void
  onOpenSettings: () => void
  onLogout: () => void
}) {
  const [adding, setAdding] = useState(false)
  const [cwdDraft, setCwdDraft] = useState('')
  const [nameDraft, setNameDraft] = useState('')
  const [busy, setBusy] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)
  const [renaming, setRenaming] = useState<RenameTarget>(null)
  const [renameDraft, setRenameDraft] = useState('')
  const [renameBusy, setRenameBusy] = useState(false)
  const [renameError, setRenameError] = useState<string | null>(null)
  const skipRenameBlurRef = useRef(false)

  useEffect(() => {
    if (!adding) {
      return
    }
    setFormError(null)
    setCwdDraft(
      suggestNewWorkspaceCwd(
        suggestedWorkspaceCwd,
        workspaces.map((w) => w.cwd),
        'project',
      ),
    )
  }, [adding, suggestedWorkspaceCwd, workspaces])

  async function submitAdd() {
    const path = cwdDraft.trim()
    if (!path) {
      setFormError('Workspace path is required')
      return
    }
    setBusy(true)
    setFormError(null)
    try {
      await onAddWorkspace(path, nameDraft.trim() || undefined)
      setAdding(false)
      setNameDraft('')
      setFormError(null)
    } catch (err) {
      setFormError(formatApiError(err))
    } finally {
      setBusy(false)
    }
  }

  function beginRenameWorkspace(ws: Workspace) {
    setAdding(false)
    setRenaming({ kind: 'workspace', id: ws.id })
    setRenameDraft(ws.name)
    setRenameError(null)
  }

  function beginRenameSession(session: Session) {
    setAdding(false)
    setRenaming({ kind: 'session', id: session.id })
    setRenameDraft(session.title || '')
    setRenameError(null)
  }

  function cancelRename() {
    skipRenameBlurRef.current = true
    setRenaming(null)
    setRenameDraft('')
    setRenameError(null)
  }

  async function submitRename() {
    if (!renaming) {
      return
    }
    const next = renameDraft.trim()
    if (!next) {
      setRenameError(
        renaming.kind === 'workspace'
          ? 'Workspace name is required'
          : 'Session title is required',
      )
      return
    }
    setRenameBusy(true)
    setRenameError(null)
    try {
      if (renaming.kind === 'workspace') {
        await onRenameWorkspace(renaming.id, next)
      } else {
        await onRenameSession(renaming.id, next)
      }
      skipRenameBlurRef.current = true
      setRenaming(null)
      setRenameDraft('')
      setRenameError(null)
    } catch (err) {
      setRenameError(formatApiError(err))
    } finally {
      setRenameBusy(false)
    }
  }

  function onRenameBlur() {
    if (skipRenameBlurRef.current) {
      skipRenameBlurRef.current = false
      return
    }
    if (!renameBusy) {
      void submitRename()
    }
  }

  return (
    <aside className="relative z-20 flex h-full min-h-0 w-64 shrink-0 flex-col border-r border-border bg-sidebar">
      <div className="flex shrink-0 items-center gap-2 px-3 py-3">
        <div className="flex h-8 w-8 items-center justify-center rounded-md bg-primary/15 font-mono text-sm font-semibold text-primary">
          zi
        </div>
        <div className="min-w-0 flex-1 overflow-hidden">
          <div className="truncate text-sm font-medium">{user.username}</div>
          <HoverTip content={<span className="font-mono">{user.id}</span>}>
            <div className="cursor-default truncate font-mono text-[10px] text-muted-foreground">
              {user.id}
            </div>
          </HoverTip>
        </div>
      </div>
      <Separator className="shrink-0" />
      <div className="relative z-30 shrink-0 overflow-visible px-3 py-2">
        <div className="mb-1 flex items-center justify-between">
          <div className="text-[10px] tracking-wide text-muted-foreground uppercase">
            Workspaces
          </div>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-7 px-2"
            onClick={() => {
              cancelRename()
              setAdding((v) => !v)
            }}
            title={adding ? 'Cancel' : 'Add workspace'}
          >
            {adding ? <X className="h-3.5 w-3.5" /> : <Plus className="h-3.5 w-3.5" />}
          </Button>
        </div>
        {adding ? (
          <div className="mb-2 space-y-2 rounded-md border border-border bg-card p-2">
            <Input
              value={nameDraft}
              onChange={(e) => setNameDraft(e.target.value)}
              placeholder="Name (optional)"
              className="h-8 text-xs"
            />
            <Input
              value={cwdDraft}
              onChange={(e) => {
                setCwdDraft(e.target.value)
                setFormError(null)
              }}
              placeholder="~/… or absolute path"
              className="h-8 font-mono text-[11px]"
              autoFocus
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  void submitAdd()
                }
                if (e.key === 'Escape') {
                  setAdding(false)
                }
              }}
            />
            <p className="text-[10px] leading-snug text-muted-foreground">
              Prefills a new folder under your ~/.zi sandbox (cwd must be
              unique).
            </p>
            {formError ? (
              <p className="text-[11px] leading-snug text-destructive">{formError}</p>
            ) : null}
            <div className="flex gap-1">
              <Button
                type="button"
                size="sm"
                className="h-7 flex-1"
                disabled={busy || !cwdDraft.trim()}
                onClick={() => {
                  void submitAdd()
                }}
              >
                Add
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-7"
                disabled={busy}
                onClick={() => setAdding(false)}
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : null}
        <div className="space-y-1">
          {workspaces.length === 0 ? (
            <button
              type="button"
              onClick={() => setAdding(true)}
              className="w-full rounded-md border border-dashed border-border px-2 py-3 text-left text-xs text-muted-foreground hover:bg-accent"
            >
              No workspaces — click to add
            </button>
          ) : (
            workspaces.map((ws) => {
              const label = workspaceLabel(ws.name, ws.cwd)
              const isRenaming =
                renaming?.kind === 'workspace' && renaming.id === ws.id
              return (
                <div
                  key={ws.id}
                  className={cn(
                    'group flex w-full items-start gap-0.5 rounded-md hover:bg-accent',
                    workspaceId === ws.id && 'bg-accent',
                  )}
                >
                  {isRenaming ? (
                    <div className="flex min-w-0 flex-1 flex-col gap-1 px-1 py-1">
                      <Input
                        value={renameDraft}
                        onChange={(e) => {
                          setRenameDraft(e.target.value)
                          setRenameError(null)
                        }}
                        className="h-7 text-xs"
                        autoFocus
                        disabled={renameBusy}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault()
                            skipRenameBlurRef.current = true
                            void submitRename()
                          }
                          if (e.key === 'Escape') {
                            cancelRename()
                          }
                        }}
                        onBlur={onRenameBlur}
                      />
                      {renameError ? (
                        <p className="px-1 text-[10px] text-destructive">
                          {renameError}
                        </p>
                      ) : null}
                    </div>
                  ) : (
                    <>
                      <HoverTip
                        content={
                          <span className="flex flex-col gap-0.5">
                            <span className="font-medium">{label}</span>
                            <span className="font-mono text-muted-foreground">
                              {ws.cwd}
                            </span>
                          </span>
                        }
                        className="min-w-0 flex-1"
                      >
                        <button
                          type="button"
                          onClick={() => onSelectWorkspace(ws.id)}
                          onDoubleClick={(e) => {
                            e.preventDefault()
                            beginRenameWorkspace(ws)
                          }}
                          className="flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left text-xs"
                        >
                          <FolderOpen className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                          <span className="min-w-0 flex-1 overflow-hidden">
                            <span className="block truncate font-medium">
                              {label}
                            </span>
                            <span className="block truncate font-mono text-[10px] text-muted-foreground">
                              {shortenPath(ws.cwd)}
                            </span>
                          </span>
                        </button>
                      </HoverTip>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="mt-0.5 h-7 w-7 shrink-0 opacity-0 group-hover:opacity-100"
                        title="Rename workspace"
                        onClick={(e) => {
                          e.stopPropagation()
                          beginRenameWorkspace(ws)
                        }}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="mt-0.5 mr-0.5 h-7 w-7 shrink-0 opacity-60 hover:opacity-100 group-hover:opacity-100"
                        title="Delete workspace"
                        onClick={(e) => {
                          e.stopPropagation()
                          onDeleteWorkspace(ws.id)
                        }}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </>
                  )}
                </div>
              )
            })
          )}
        </div>
      </div>
      <Separator className="shrink-0" />
      <div className="relative z-10 flex shrink-0 items-center justify-between px-3 py-2">
        <div className="text-[10px] tracking-wide text-muted-foreground uppercase">
          Sessions
        </div>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="relative z-10 h-7 px-2"
          onClick={onNewSession}
          title={
            workspaceId
              ? 'New session'
              : 'New session (will create a default workspace if needed)'
          }
        >
          <MessageSquarePlus className="h-3.5 w-3.5" />
        </Button>
      </div>
      <ScrollArea className="min-h-0 flex-1 px-2">
        <div className="space-y-1 pb-3">
          {sessions.map((session) => {
            const title = session.title || 'Untitled'
            const isRenaming =
              renaming?.kind === 'session' && renaming.id === session.id
            return (
              <div
                key={session.id}
                className={cn(
                  'group flex w-full items-start gap-1 rounded-md hover:bg-accent',
                  sessionId === session.id && 'bg-accent',
                )}
              >
                {isRenaming ? (
                  <div className="flex min-w-0 flex-1 flex-col gap-1 px-1 py-1">
                    <Input
                      value={renameDraft}
                      onChange={(e) => {
                        setRenameDraft(e.target.value)
                        setRenameError(null)
                      }}
                      className="h-7 text-xs"
                      autoFocus
                      disabled={renameBusy}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.preventDefault()
                          skipRenameBlurRef.current = true
                          void submitRename()
                        }
                        if (e.key === 'Escape') {
                          cancelRename()
                        }
                      }}
                      onBlur={onRenameBlur}
                    />
                    {renameError ? (
                      <p className="px-1 text-[10px] text-destructive">
                        {renameError}
                      </p>
                    ) : null}
                  </div>
                ) : (
                  <>
                    <HoverTip content={title} className="min-w-0 flex-1">
                      <button
                        type="button"
                        onClick={() => onSelectSession(session.id)}
                        onDoubleClick={(e) => {
                          e.preventDefault()
                          beginRenameSession(session)
                        }}
                        className="flex w-full min-w-0 flex-col gap-1 px-2 py-2 text-left"
                      >
                        <span className="block truncate text-sm">{title}</span>
                        <span className="mt-1 flex items-center gap-2">
                          <Badge
                            variant="outline"
                            className="font-mono text-[10px]"
                          >
                            {session.model || 'model'}
                          </Badge>
                          <span className="text-[10px] text-muted-foreground">
                            r{session.revision}
                          </span>
                        </span>
                      </button>
                    </HoverTip>
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="mt-1 h-7 w-7 shrink-0 opacity-0 group-hover:opacity-100"
                      title="Rename session"
                      onClick={(e) => {
                        e.stopPropagation()
                        beginRenameSession(session)
                      }}
                    >
                      <Pencil className="h-3.5 w-3.5" />
                    </Button>
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="mt-1 mr-1 h-7 w-7 shrink-0 opacity-60 hover:opacity-100 group-hover:opacity-100"
                      title="Delete session"
                      onClick={(e) => {
                        e.stopPropagation()
                        onDeleteSession(session.id)
                      }}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </>
                )}
              </div>
            )
          })}
          {sessions.length === 0 ? (
            <div className="px-2 py-4 text-xs text-muted-foreground">
              No sessions
            </div>
          ) : null}
        </div>
      </ScrollArea>
      <Separator className="shrink-0" />
      <div className="flex shrink-0 gap-1 p-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="flex-1 justify-start"
          onClick={onOpenSettings}
        >
          <Settings className="h-3.5 w-3.5" />
          Settings
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          onClick={onLogout}
          title="Logout"
        >
          <LogOut className="h-3.5 w-3.5" />
        </Button>
      </div>
    </aside>
  )
}
