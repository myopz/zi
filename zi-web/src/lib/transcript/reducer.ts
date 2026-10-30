import type {
  AgentEvent,
  AgentMessage,
  ToolRunState,
  TranscriptItem,
  TranscriptState,
} from '@/lib/types'

/**
 * Wire JSON sometimes used PascalCase role values (`Assistant`) when the Rust
 * unit-role enum was serialized beside the Message tag. Normalize for the UI.
 */
function normalizeRole(role: unknown): string {
  const raw = String(role ?? '')
  const lower = raw.toLowerCase()
  if (lower === 'toolresult') {
    return 'toolResult'
  }
  return lower
}

function withNormalizedRole<T extends { role: string }>(message: T): T {
  const role = normalizeRole(message.role)
  if (role === message.role) {
    return message
  }
  return { ...message, role }
}

function textFromUserContent(content: Extract<AgentMessage, { role: 'user' }>['content']): string {
  if (typeof content === 'string') {
    return content
  }
  return content
    .map((block) => ('text' in block ? block.text : '[image]'))
    .join('\n')
}

function toolResultText(message: Extract<AgentMessage, { role: 'toolResult' }>): string {
  return message.content
    .map((block) => ('text' in block ? block.text : '[image]'))
    .join('\n')
}

export function messagesToTranscript(messages: AgentMessage[]): TranscriptItem[] {
  const items: TranscriptItem[] = []
  for (const [index, raw] of messages.entries()) {
    const message = withNormalizedRole(raw)
    if (message.role === 'user') {
      items.push({
        kind: 'user',
        id: `user-${message.timestamp}-${index}`,
        text: textFromUserContent(message.content),
        timestamp: message.timestamp,
      })
      continue
    }
    if (message.role === 'assistant') {
      items.push({
        kind: 'assistant',
        id: `assistant-${message.timestamp}-${index}`,
        message: message as Extract<AgentMessage, { role: 'assistant' }>,
        streaming: false,
      })
      continue
    }
    if (message.role === 'toolResult') {
      const toolMessage = message as Extract<AgentMessage, { role: 'toolResult' }>
      items.push({
        kind: 'tool',
        id: `tool-${toolMessage.toolCallId}`,
        run: {
          toolCallId: toolMessage.toolCallId,
          toolName: toolMessage.toolName,
          args: null,
          partialText: '',
          resultText: toolResultText(toolMessage),
          isError: toolMessage.isError,
          status: toolMessage.isError ? 'error' : 'done',
        },
      })
      continue
    }
    if (message.role === 'system') {
      const text =
        typeof message.content === 'string'
          ? message.content
          : message.content.map((b) => b.text).join('\n')
      items.push({
        kind: 'system',
        id: `system-${message.timestamp}-${index}`,
        text,
        timestamp: message.timestamp,
      })
    }
  }
  return collapseIdenticalToolResults(items)
}

export function createTranscriptState(
  messages: AgentMessage[] = [],
  revision = 0,
): TranscriptState {
  return {
    items: messagesToTranscript(messages),
    busy: false,
    revision,
    contextUsage: undefined,
  }
}

export function transcriptToMessages(state: TranscriptState): AgentMessage[] {
  const messages: AgentMessage[] = []
  for (const item of state.items) {
    if (item.kind === 'user') {
      messages.push({
        role: 'user',
        content: item.text,
        timestamp: item.timestamp,
      })
    } else if (item.kind === 'assistant') {
      messages.push(item.message)
    } else if (item.kind === 'tool' && item.run.status !== 'running') {
      messages.push({
        role: 'toolResult',
        toolCallId: item.run.toolCallId,
        toolName: item.run.toolName,
        content: [{ text: item.run.resultText ?? item.run.partialText }],
        isError: item.run.isError ?? false,
        timestamp: Date.now(),
      })
    } else if (item.kind === 'system') {
      messages.push({
        role: 'system',
        content: item.text,
        timestamp: item.timestamp,
      })
    }
  }
  return messages
}

function upsertTool(items: TranscriptItem[], run: ToolRunState): TranscriptItem[] {
  const id = `tool-${run.toolCallId}`
  const index = items.findIndex((item) => item.kind === 'tool' && item.id === id)
  const next: TranscriptItem = { kind: 'tool', id, run }
  if (index === -1) {
    return collapseIdenticalToolResults([...items, next])
  }
  const copy = items.slice()
  copy[index] = next
  return collapseIdenticalToolResults(copy)
}

/** Path from `Updated PATH (…)` / `Wrote PATH (…)` success lines. */
function mutationResultPath(text: string): string | null {
  const line = text.split('\n')[0]?.trim() ?? ''
  const rest = line.startsWith('Updated ')
    ? line.slice('Updated '.length)
    : line.startsWith('Wrote ')
      ? line.slice('Wrote '.length)
      : null
  if (!rest) {
    return null
  }
  const cut = rest.lastIndexOf(' (')
  return (cut >= 0 ? rest.slice(0, cut) : rest).trim() || null
}

function sameMutationPath(a: string, b: string, toolName: string): boolean {
  if (toolName !== 'edit' && toolName !== 'write') {
    return false
  }
  const pa = mutationResultPath(a)
  const pb = mutationResultPath(b)
  return Boolean(pa && pb && pa === pb)
}

/**
 * Merge consecutive finished tool cards that share the same name + result text
 * (or same edit/write path). Stops the transcript from flooding polish loops.
 */
export function collapseIdenticalToolResults(
  items: TranscriptItem[],
): TranscriptItem[] {
  const out: TranscriptItem[] = []
  for (const item of items) {
    if (item.kind !== 'tool' || item.run.status === 'running') {
      out.push(item)
      continue
    }
    const prev = out[out.length - 1]
    const prevText = prev?.kind === 'tool' ? (prev.run.resultText ?? '') : ''
    const curText = item.run.resultText ?? ''
    const sameText =
      prevText === curText && curText.length > 0
    const samePath =
      prev?.kind === 'tool' &&
      sameMutationPath(prevText, curText, item.run.toolName)
    if (
      prev?.kind === 'tool' &&
      prev.run.status !== 'running' &&
      prev.run.toolName === item.run.toolName &&
      Boolean(prev.run.isError) === Boolean(item.run.isError) &&
      (sameText || samePath)
    ) {
      out[out.length - 1] = {
        ...prev,
        run: {
          ...prev.run,
          // Keep the latest success line so N edit(s) stays current.
          resultText: curText || prev.run.resultText,
          repeatCount: (prev.run.repeatCount ?? 1) + (item.run.repeatCount ?? 1),
          durationMs: item.run.durationMs ?? prev.run.durationMs,
        },
      }
      continue
    }
    out.push(item)
  }
  return out
}

function replaceStreamingAssistant(
  items: TranscriptItem[],
  message: Extract<AgentMessage, { role: 'assistant' }>,
  streaming: boolean,
): TranscriptItem[] {
  const streamingIndex = items.findIndex(
    (item) => item.kind === 'assistant' && item.streaming,
  )
  const next: TranscriptItem = {
    kind: 'assistant',
    id: `assistant-${message.timestamp}-live`,
    message,
    streaming,
  }
  if (streamingIndex >= 0) {
    const copy = items.slice()
    copy[streamingIndex] = next
    return copy
  }
  return [...items, next]
}

export function reduceAgentEvent(
  state: TranscriptState,
  event: AgentEvent,
): TranscriptState {
  switch (event.type) {
    case 'agent_start':
      return { ...state, busy: true }
    case 'agent_end':
      // `messages` is only this run's delta (prompt + replies), not full history.
      // Replacing items here wiped earlier turns after the second prompt.
      return {
        ...state,
        busy: false,
        items: state.items.map((item) =>
          item.kind === 'assistant' ? { ...item, streaming: false } : item,
        ),
      }
    case 'turn_start':
      return { ...state, busy: true }
    case 'turn_end':
      return { ...state, busy: true }
    case 'message_start': {
      const message = withNormalizedRole(event.message)
      if (message.role !== 'assistant') {
        return state
      }
      return {
        ...state,
        items: replaceStreamingAssistant(
          state.items,
          message as Extract<AgentMessage, { role: 'assistant' }>,
          true,
        ),
      }
    }
    case 'message_update': {
      const message = withNormalizedRole(event.message)
      if (message.role !== 'assistant') {
        return state
      }
      return {
        ...state,
        items: replaceStreamingAssistant(
          state.items,
          message as Extract<AgentMessage, { role: 'assistant' }>,
          true,
        ),
      }
    }
    case 'message_end': {
      const message = withNormalizedRole(event.message)
      if (message.role === 'user') {
        const text = textFromUserContent(
          (message as Extract<AgentMessage, { role: 'user' }>).content,
        )
        // Skip duplicate when the composer already appended an optimistic user row.
        const last = state.items[state.items.length - 1]
        if (
          last?.kind === 'user' &&
          last.text === text &&
          Math.abs(last.timestamp - message.timestamp) < 60_000
        ) {
          return state
        }
        return {
          ...state,
          items: [
            ...state.items,
            {
              kind: 'user',
              id: `user-${message.timestamp}-end`,
              text,
              timestamp: message.timestamp,
            },
          ],
        }
      }
      if (message.role === 'assistant') {
        return {
          ...state,
          items: replaceStreamingAssistant(
            state.items,
            message as Extract<AgentMessage, { role: 'assistant' }>,
            false,
          ),
        }
      }
      if (message.role === 'toolResult') {
        const toolMessage = message as Extract<
          AgentMessage,
          { role: 'toolResult' }
        >
        return {
          ...state,
          items: upsertTool(state.items, {
            toolCallId: toolMessage.toolCallId,
            toolName: toolMessage.toolName,
            args: null,
            partialText: '',
            resultText: toolResultText(toolMessage),
            isError: toolMessage.isError,
            status: toolMessage.isError ? 'error' : 'done',
          }),
        }
      }
      return state
    }
    case 'tool_execution_start':
      return {
        ...state,
        items: upsertTool(state.items, {
          toolCallId: event.tool_call_id,
          toolName: event.tool_name,
          args: event.args,
          partialText: '',
          status: 'running',
        }),
      }
    case 'tool_execution_update': {
      const text = event.partial_result.content.map((c) => c.text).join('\n')
      const existing = state.items.find(
        (item) =>
          item.kind === 'tool' && item.run.toolCallId === event.tool_call_id,
      )
      const prev =
        existing && existing.kind === 'tool'
          ? existing.run
          : {
              toolCallId: event.tool_call_id,
              toolName: event.tool_name,
              args: event.args,
              partialText: '',
              status: 'running' as const,
            }
      return {
        ...state,
        items: upsertTool(state.items, {
          ...prev,
          args: event.args,
          toolName: event.tool_name,
          partialText: text,
          status: 'running',
        }),
      }
    }
    case 'tool_execution_end': {
      const text = event.result.content.map((c) => c.text).join('\n')
      const existing = state.items.find(
        (item) =>
          item.kind === 'tool' && item.run.toolCallId === event.tool_call_id,
      )
      const prevArgs =
        existing && existing.kind === 'tool' ? existing.run.args : null
      return {
        ...state,
        items: upsertTool(state.items, {
          toolCallId: event.tool_call_id,
          toolName: event.tool_name,
          args: prevArgs,
          partialText: text,
          resultText: text,
          isError: event.is_error,
          status: event.is_error ? 'error' : 'done',
          durationMs: event.duration_ms,
        }),
      }
    }
    case 'context_usage':
      return {
        ...state,
        contextUsage: {
          estimatedTokens: event.estimated_tokens,
          contextWindow: event.context_window,
          compactLimit: event.compact_limit,
          foldedToolResults: event.folded_tool_results,
          cumulativeInput: event.cumulative_input,
          cumulativeOutput: event.cumulative_output,
          nearLimit: event.near_limit,
        },
      }
    case 'compaction':
      return {
        ...state,
        contextUsage: {
          estimatedTokens: event.tokens_after,
          contextWindow: state.contextUsage?.contextWindow ?? 0,
          compactLimit: state.contextUsage?.compactLimit ?? 0,
          foldedToolResults: 0,
          cumulativeInput: state.contextUsage?.cumulativeInput ?? 0,
          cumulativeOutput: state.contextUsage?.cumulativeOutput ?? 0,
          nearLimit: false,
        },
      }
    default:
      return state
  }
}

export function appendOptimisticUser(
  state: TranscriptState,
  text: string,
): TranscriptState {
  return {
    ...state,
    busy: true,
    items: [
      ...state.items,
      {
        kind: 'user',
        id: `user-local-${Date.now()}`,
        text,
        timestamp: Date.now(),
      },
    ],
  }
}

/** Apply a local slash-command result (does not go through the LLM). */
export function applySlashResult(
  state: TranscriptState,
  result: {
    kind: 'notice' | 'reset'
    text: string
    revision?: number
    seed?: Array<{ role: string; content: string }>
  },
): TranscriptState {
  const now = Date.now()
  if (result.kind === 'reset') {
    const seedItems: TranscriptItem[] = (result.seed ?? []).map((row, index) => {
      const role = row.role.toLowerCase()
      if (role === 'user') {
        return {
          kind: 'user' as const,
          id: `slash-seed-user-${now}-${index}`,
          text: row.content,
          timestamp: now,
        }
      }
      if (role === 'assistant') {
        return {
          kind: 'system' as const,
          id: `slash-seed-assistant-${now}-${index}`,
          text: row.content,
          timestamp: now,
        }
      }
      return {
        kind: 'system' as const,
        id: `slash-seed-${now}-${index}`,
        text: row.content,
        timestamp: now,
      }
    })
    return {
      items: [
        ...seedItems,
        {
          kind: 'system',
          id: `slash-notice-${now}`,
          text: result.text,
          timestamp: now,
        },
      ],
      busy: false,
      revision: result.revision ?? state.revision,
      contextUsage: undefined,
    }
  }
  return {
    ...state,
    busy: false,
    revision: result.revision ?? state.revision,
    items: [
      ...state.items,
      {
        kind: 'system',
        id: `slash-notice-${now}`,
        text: result.text,
        timestamp: now,
      },
    ],
  }
}
