import { LoaderCircle, Square, SendHorizontal } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Textarea } from '@/components/ui/textarea'
import type { ModelInfo } from '@/lib/types'

export function Composer({
  draft,
  onDraftChange,
  models,
  model,
  onModelChange,
  busy,
  connected,
  disabled,
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
  onSubmit: (text: string) => void
  onAbort: () => void
}) {
  const [local, setLocal] = useState(draft)

  useEffect(() => {
    setLocal(draft)
  }, [draft])

  function update(value: string) {
    setLocal(value)
    onDraftChange(value)
  }

  function submit() {
    const text = local.trim()
    if (!text || busy || disabled) {
      return
    }
    onSubmit(text)
  }

  return (
    <div className="border-t border-border bg-card/60 p-3">
      <div className="mx-auto flex max-w-3xl flex-col gap-2">
        <div className="flex items-center gap-2">
          <div className="w-56">
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
          <div className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
            <span
              className={
                connected ? 'text-emerald-400' : 'text-amber-400'
              }
            >
              {connected ? 'ws connected' : 'ws reconnecting'}
            </span>
            {busy ? (
              <span className="inline-flex items-center gap-1">
                <LoaderCircle className="h-3 w-3 animate-spin" />
                running
              </span>
            ) : null}
          </div>
        </div>
        <div className="flex gap-2">
          <Textarea
            value={local}
            onChange={(e) => update(e.target.value)}
            placeholder={
              disabled
                ? 'Create a workspace and session to chat…'
                : 'Message zi…'
            }
            className="min-h-[88px] flex-1 resize-none"
            // Keep focusable/editable so the box never looks "dead"; send still
            // gates on session readiness.
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                submit()
              }
            }}
          />
          <div className="flex flex-col gap-2">
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
