import { ChevronRight, LoaderCircle, Wrench } from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/components/ui/badge'
import type { ToolRunState } from '@/lib/types'
import { cn } from '@/lib/utils'

function formatArgs(args: unknown): string {
  if (args == null) {
    return ''
  }
  if (typeof args === 'string') {
    return args
  }
  try {
    return JSON.stringify(args, null, 2)
  } catch {
    return String(args)
  }
}

export function ToolCard({ run }: { run: ToolRunState }) {
  const [open, setOpen] = useState(run.status === 'running')
  const body = run.resultText ?? run.partialText
  return (
    <div className="rounded-md border border-border bg-card/80">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm"
        onClick={() => setOpen((v) => !v)}
      >
        <ChevronRight
          className={cn('h-3.5 w-3.5 text-muted-foreground transition-transform', open && 'rotate-90')}
        />
        <Wrench className="h-3.5 w-3.5 text-primary" />
        <span className="font-mono text-xs">{run.toolName}</span>
        <div className="ml-auto flex items-center gap-2">
          {run.status === 'running' ? (
            <LoaderCircle className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
          ) : null}
          <Badge
            variant={
              run.status === 'error'
                ? 'destructive'
                : run.status === 'running'
                  ? 'default'
                  : 'secondary'
            }
          >
            {run.status}
          </Badge>
          {run.durationMs != null ? (
            <span className="text-[10px] text-muted-foreground">
              {run.durationMs}ms
            </span>
          ) : null}
        </div>
      </button>
      {open ? (
        <div className="space-y-2 border-t border-border px-3 py-2">
          {run.args != null ? (
            <div>
              <div className="mb-1 text-[10px] tracking-wide text-muted-foreground uppercase">
                args
              </div>
              <pre className="max-h-40 overflow-auto rounded bg-muted p-2 font-mono text-xs whitespace-pre-wrap">
                {formatArgs(run.args)}
              </pre>
            </div>
          ) : null}
          {body ? (
            <div>
              <div className="mb-1 text-[10px] tracking-wide text-muted-foreground uppercase">
                result
              </div>
              <pre className="max-h-56 overflow-auto rounded bg-muted p-2 font-mono text-xs whitespace-pre-wrap">
                {body}
              </pre>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
