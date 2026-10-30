import { LoaderCircle, Square, SendHorizontal } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { ContextRing } from '@/components/chat/ContextRing'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Textarea } from '@/components/ui/textarea'
import { listSlashCommands } from '@/lib/api'
import {
  FALLBACK_SLASH_COMMANDS,
  filterSlashCommands,
  parseSlashQuery,
} from '@/lib/slash-commands'
import type { ContextUsageSnapshot, ModelInfo, SlashCommand } from '@/lib/types'
import { formatTokens } from '@/lib/usage'
import { cn } from '@/lib/utils'

export function Composer({
  draft,
  onDraftChange,
  models,
  model,
  onModelChange,
  busy,
  connected,
  disabled,
  workspaceId,
  contextUsage,
  sessionTokens,
  onSubmit,
  onAbort,
}: {
  draft: string
  onDraftChange: (value: string) => void
  models: ModelInfo[]
  model: string
  onModelChange: (model: string) => void
  busy: boolean
  connected: boolean
  disabled?: boolean
  workspaceId?: string | null
  contextUsage?: ContextUsageSnapshot
  /** Running Σ from finished assistant turns (for diagnosis). */
  sessionTokens?: { input: number; output: number; turns: number }
  onSubmit: (text: string) => void
  onAbort: () => void
}) {
  const [local, setLocal] = useState(draft)
  const [commands, setCommands] = useState<SlashCommand[]>(
    FALLBACK_SLASH_COMMANDS,
  )
  const [activeIndex, setActiveIndex] = useState(0)
  const [menuDismissed, setMenuDismissed] = useState(false)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([])

  useEffect(() => {
    setLocal(draft)
  }, [draft])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const remote = await listSlashCommands(workspaceId)
        if (!cancelled && remote.length > 0) {
          setCommands(remote)
        }
      } catch {
        if (!cancelled) {
          setCommands(FALLBACK_SLASH_COMMANDS)
        }
      }
    })()
    return () => {
      cancelled = true
    }
  }, [workspaceId])

  const slashQuery = parseSlashQuery(local)
  const filtered = useMemo(
    () =>
      slashQuery === null
        ? []
        : filterSlashCommands(commands, slashQuery),
    [commands, slashQuery],
  )
  const menuOpen =
    !disabled && !menuDismissed && slashQuery !== null && filtered.length > 0

  useEffect(() => {
    setActiveIndex(0)
    setMenuDismissed(false)
  }, [slashQuery])

  useEffect(() => {
    if (!menuOpen) {
      return
    }
    itemRefs.current[activeIndex]?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex, menuOpen])

  function update(value: string) {
    setLocal(value)
    onDraftChange(value)
  }

  function applySlash(cmd: SlashCommand) {
    const next = `/${cmd.name} `
    update(next)
    setMenuDismissed(true)
    requestAnimationFrame(() => {
      const el = textareaRef.current
      if (!el) {
        return
      }
      el.focus()
      const pos = next.length
      el.setSelectionRange(pos, pos)
    })
  }

  function submit() {
    const text = local.trim()
    if (!text || busy || disabled) {
      return
    }
    onSubmit(text)
  }

  return (
    <div className="border-t border-border bg-card/60 p-2 sm:p-3 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
      <div className="mx-auto flex max-w-3xl flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <div className="min-w-0 flex-1 sm:w-56 sm:flex-none">
            <Select
              value={model || undefined}
              onValueChange={onModelChange}
              disabled={busy || models.length === 0}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select model" />
              </SelectTrigger>
              <SelectContent>
                {models.map((m) => (
                  <SelectItem key={m.id} value={m.id}>
                    {m.name ?? m.id}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="ml-auto flex flex-wrap items-center justify-end gap-2 text-xs text-muted-foreground">
            {sessionTokens && sessionTokens.turns > 0 ? (
              <span
                className="font-mono tabular-nums"
                title={`Session Σ input ${sessionTokens.input.toLocaleString()} · output ${sessionTokens.output.toLocaleString()} across ${sessionTokens.turns} model turns`}
              >
                Σ {formatTokens(sessionTokens.input)}
                <span className="hidden sm:inline">
                  {' '}
                  in · {formatTokens(sessionTokens.output)} out
                </span>
              </span>
            ) : null}
            <span
              className={
                connected ? 'text-emerald-400' : 'text-amber-400'
              }
            >
              <span className="sm:hidden">{connected ? 'ws' : '…'}</span>
              <span className="hidden sm:inline">
                {connected ? 'ws connected' : 'ws reconnecting'}
              </span>
            </span>
            {busy ? (
              <span className="inline-flex items-center gap-1">
                <LoaderCircle className="h-3 w-3 animate-spin" />
                <span className="hidden sm:inline">running</span>
              </span>
            ) : null}
          </div>
        </div>
        <div className="relative flex gap-2">
          {menuOpen ? (
            <div
              className="absolute bottom-full left-0 z-40 mb-2 w-full max-w-md overflow-hidden rounded-md border border-border bg-popover text-popover-foreground shadow-lg"
              role="listbox"
              aria-label="Slash commands"
            >
              <div className="border-b border-border px-3 py-1.5 text-[10px] tracking-wide text-muted-foreground uppercase">
                Commands
              </div>
              <div className="max-h-64 overflow-y-auto p-1">
                {filtered.map((cmd, index) => (
                  <button
                    key={cmd.name}
                    type="button"
                    role="option"
                    aria-selected={index === activeIndex}
                    ref={(el) => {
                      itemRefs.current[index] = el
                    }}
                    className={cn(
                      'flex w-full items-start gap-3 rounded-sm px-2 py-1.5 text-left text-sm',
                      index === activeIndex
                        ? 'bg-accent text-accent-foreground'
                        : 'hover:bg-accent/60',
                    )}
                    onMouseEnter={() => setActiveIndex(index)}
                    onMouseDown={(e) => {
                      // Prevent textarea blur before click applies.
                      e.preventDefault()
                    }}
                    onClick={() => applySlash(cmd)}
                  >
                    <span className="shrink-0 font-mono text-primary">
                      /{cmd.name}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                      {cmd.summary}
                    </span>
                  </button>
                ))}
              </div>
              <div className="border-t border-border px-3 py-1.5 text-[10px] text-muted-foreground">
                ↑↓ navigate · Tab/Enter select · Esc dismiss
              </div>
            </div>
          ) : null}
          <Textarea
            ref={textareaRef}
            value={local}
            onChange={(e) => update(e.target.value)}
            placeholder={
              disabled
                ? 'Create a workspace and session to chat…'
                : 'Message zi…  (/ for commands)'
            }
            className="min-h-[88px] flex-1 resize-none"
            // Keep focusable/editable so the box never looks "dead"; send still
            // gates on session readiness.
            onKeyDown={(e) => {
              if (menuOpen) {
                if (e.key === 'ArrowDown') {
                  e.preventDefault()
                  setActiveIndex((i) =>
                    filtered.length === 0 ? 0 : (i + 1) % filtered.length,
                  )
                  return
                }
                if (e.key === 'ArrowUp') {
                  e.preventDefault()
                  setActiveIndex((i) =>
                    filtered.length === 0
                      ? 0
                      : (i - 1 + filtered.length) % filtered.length,
                  )
                  return
                }
                if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
                  const cmd = filtered[activeIndex]
                  if (cmd) {
                    e.preventDefault()
                    applySlash(cmd)
                  }
                  return
                }
                if (e.key === 'Escape') {
                  e.preventDefault()
                  setMenuDismissed(true)
                  return
                }
              }
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                submit()
              }
            }}
          />
          <div className="flex flex-col items-center gap-2">
            <ContextRing usage={contextUsage} />
            {busy ? (
              <Button
                type="button"
                variant="destructive"
                size="icon"
                onClick={onAbort}
                title="Abort"
              >
                <Square className="h-4 w-4" />
              </Button>
            ) : (
              <Button
                type="button"
                size="icon"
                onClick={submit}
                disabled={disabled || !local.trim()}
                title={disabled ? 'Select or create a session first' : 'Send'}
              >
                <SendHorizontal className="h-4 w-4" />
              </Button>
            )}
          </div>
        </div>
        {disabled ? (
          <p className="text-xs text-muted-foreground">
            No active session — add a workspace in the sidebar, then New session.
          </p>
        ) : null}
      </div>
    </div>
  )
}
