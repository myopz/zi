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
  return items
}

export function createTranscriptState(
  messages: AgentMessage[] = [],
  revision = 0,
): TranscriptState {
  return {
    items: messagesToTranscript(messages),
    busy: false,
    revision,
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
    return [...items, next]
  }
  const copy = items.slice()
  copy[index] = next
  return copy
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
      return {
        ...state,
        busy: false,
        items: messagesToTranscript(event.messages).length
          ? messagesToTranscript(event.messages)
          : state.items.map((item) =>
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
