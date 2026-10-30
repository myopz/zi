import { MarkdownBlock } from '@/components/chat/MarkdownBlock'
import { ThinkingBlock } from '@/components/chat/ThinkingBlock'
import { ToolCard } from '@/components/chat/ToolCard'
import { ScrollArea } from '@/components/ui/scroll-area'
import { toolStepLabel } from '@/lib/tool-labels'
import type { TranscriptItem } from '@/lib/types'
import { cn } from '@/lib/utils'
import {
  formatTokens,
  parseAssistantUsage,
  sumTranscriptUsage,
} from '@/lib/usage'

function AssistantBlocks({
  item,
  toolCardIds,
}: {
  item: Extract<TranscriptItem, { kind: 'assistant' }>
  /** toolCall ids that already have a ToolCard — hide the dashed placeholder. */
  toolCardIds: Set<string>
}) {
  return (
    <div className="space-y-2">
      {item.message.content.map((block, index) => {
        if (block.type === 'text') {
          return <MarkdownBlock key={`t-${index}`} text={block.text} />
        }
        if (block.type === 'thinking') {
          return (
            <ThinkingBlock
              key={`th-${index}`}
              thinking={block.thinking}
              streaming={item.streaming}
            />
          )
        }
        if (block.type === 'toolCall') {
          if (block.id && toolCardIds.has(block.id)) {
            return null
          }
          return (
            <div
              key={`tc-${block.id || index}`}
              className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground"
            >
              {toolStepLabel(block.name)}
            </div>
          )
        }
        return null
      })}
      {item.message.errorMessage ? (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {item.message.errorMessage}
        </div>
      ) : null}
    </div>
  )
}

function TurnUsageLabel({
  item,
}: {
  item: Extract<TranscriptItem, { kind: 'assistant' }>
}) {
  if (item.streaming) {
    return (
      <span className="font-mono text-[10px] text-muted-foreground">
        tokens…
      </span>
    )
  }
  const u = parseAssistantUsage(item.message.usage)
  if (!u || (u.input === 0 && u.output === 0 && u.totalTokens === 0)) {
    return null
  }
  const tip = [
    `input ${u.input.toLocaleString()}`,
    `output ${u.output.toLocaleString()}`,
    u.cacheRead ? `cacheRead ${u.cacheRead.toLocaleString()}` : null,
    u.cacheWrite ? `cacheWrite ${u.cacheWrite.toLocaleString()}` : null,
    `total ${u.totalTokens.toLocaleString()}`,
  ]
    .filter(Boolean)
    .join(' · ')
  return (
    <span
      className="font-mono text-[10px] tabular-nums text-muted-foreground"
      title={tip}
    >
      in {formatTokens(u.input)} · out {formatTokens(u.output)}
      {u.cacheRead > 0 ? ` · cache ${formatTokens(u.cacheRead)}` : ''}
    </span>
  )
}

export function Transcript({
  items,
  className,
}: {
  items: TranscriptItem[]
  className?: string
}) {
  const totals = sumTranscriptUsage(items)
  const toolCardIds = new Set(
    items
      .filter((item): item is Extract<TranscriptItem, { kind: 'tool' }> => item.kind === 'tool')
      .map((item) => item.run.toolCallId),
  )

  return (
    <ScrollArea className={cn('h-full', className)}>
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6">
        {totals.turns > 0 ? (
          <div
            className="sticky top-0 z-10 -mx-4 border-b border-border/80 bg-background/95 px-4 py-2 backdrop-blur supports-[backdrop-filter]:bg-background/80"
            title={`Σ input ${totals.input.toLocaleString()} · output ${totals.output.toLocaleString()} · cacheRead ${totals.cacheRead.toLocaleString()} · ${totals.turns} model turns`}
          >
            <div className="flex flex-wrap items-baseline justify-between gap-2 text-xs">
              <span className="text-muted-foreground">Session tokens</span>
              <span className="font-mono tabular-nums text-foreground">
                Σ in {formatTokens(totals.input)} · out{' '}
                {formatTokens(totals.output)}
                {totals.cacheRead > 0
                  ? ` · cache ${formatTokens(totals.cacheRead)}`
                  : ''}
                <span className="text-muted-foreground">
                  {' '}
                  · {totals.turns} turns · bill{' '}
                  {formatTokens(totals.totalTokens)}
                </span>
              </span>
            </div>
          </div>
        ) : null}
        {items.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border px-4 py-10 text-center text-sm text-muted-foreground">
            Start a conversation. Prompts stream over WebSocket as AgentEvents.
          </div>
        ) : null}
        {items.map((item) => {
          if (item.kind === 'user') {
            return (
              <div key={item.id} className="flex justify-end">
                <div className="max-w-[85%] rounded-lg bg-primary/15 px-3 py-2 text-sm whitespace-pre-wrap">
                  {item.text}
                </div>
              </div>
            )
          }
          if (item.kind === 'assistant') {
            return (
              <div key={item.id} className="space-y-2">
                <div className="flex flex-wrap items-baseline justify-between gap-2">
                  <div className="text-[10px] tracking-wide text-muted-foreground uppercase">
                    assistant
                    {item.streaming ? ' · streaming' : ''}
                    {item.message.model ? ` · ${item.message.model}` : ''}
                  </div>
                  <TurnUsageLabel item={item} />
                </div>
                <AssistantBlocks item={item} toolCardIds={toolCardIds} />
              </div>
            )
          }
          if (item.kind === 'tool') {
            return <ToolCard key={item.id} run={item.run} />
          }
          return (
            <div
              key={item.id}
              className="rounded-md border border-border/60 bg-muted/30 px-3 py-2 text-xs text-muted-foreground"
            >
              {item.text}
            </div>
          )
        })}
      </div>
    </ScrollArea>
  )
}
