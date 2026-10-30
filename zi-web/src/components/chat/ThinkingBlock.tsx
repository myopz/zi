import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'

export function ThinkingBlock({
  thinking,
  streaming,
}: {
  thinking: string
  streaming?: boolean
}) {
  const [open, setOpen] = useState(streaming ?? false)
  return (
    <div className="rounded-md border border-border/70 bg-muted/40">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-muted-foreground hover:text-foreground"
        onClick={() => setOpen((v) => !v)}
      >
        <ChevronRight
          className={cn('h-3.5 w-3.5 transition-transform', open && 'rotate-90')}
        />
        <span>{streaming ? 'Thought…' : 'Thought'}</span>
      </button>
      {open ? (
        <pre className="max-h-64 overflow-auto border-t border-border/70 px-3 py-2 font-mono text-xs whitespace-pre-wrap text-muted-foreground">
          {thinking || (streaming ? '…' : '')}
        </pre>
      ) : null}
    </div>
  )
}
