import { MarkdownBlock } from '@/components/chat/MarkdownBlock'
import { ThinkingBlock } from '@/components/chat/ThinkingBlock'
import { ToolCard } from '@/components/chat/ToolCard'
import { ScrollArea } from '@/components/ui/scroll-area'
import type { TranscriptItem } from '@/lib/types'
import { cn } from '@/lib/utils'

function AssistantBlocks({
  item,
}: {
  item: Extract<TranscriptItem, { kind: 'assistant' }>
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
          return (
            <div
              key={`tc-${block.id || index}`}
              className="rounded-md border border-dashed border-border px-3 py-2 font-mono text-xs text-muted-foreground"
            >
              toolCall {block.name}
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

export function Transcript({
  items,
  className,
}: {
  items: TranscriptItem[]
  className?: string
}) {
  return (
    <ScrollArea className={cn('h-full', className)}>
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6">
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
                <div className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  assistant
                  {item.streaming ? ' · streaming' : ''}
                  {item.message.model ? ` · ${item.message.model}` : ''}
                </div>
                <AssistantBlocks item={item} />
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
