/** Shared API / WS / transcript types aligned with the zi-web plan contract. */

export type User = {
  id: string
  username: string
}

export type DefaultsResponse = {
  /** `$ZI_AGENT_DIR/web/workspaces/<userId>` — under ~/.zi, never the git repo. */
  defaultWorkspaceCwd: string
}

export type LoginResponse = {
  accessToken: string
  user: User
}

export type Workspace = {
  id: string
  userId: string
  cwd: string
  name: string
  updatedAt: string
}

export type Session = {
  id: string
  userId: string
  workspaceId: string
  title: string
  model: string
  createdAt: string
  updatedAt: string
  revision: number
}

export type ModelInfo = {
  id: string
  provider?: string
  name?: string
}

export type ModelsResponse = {
  models: ModelInfo[]
  current?: string
}

export type SessionStatus = {
  sessionId?: string
  busy: boolean
  model?: string
  error?: string
}

export type StoredMessage = {
  id: string
  sessionId: string
  role: string
  contentJson: unknown
  createdAt: string
  seq: number
}

export type SessionDetail = {
  session: Session
  messages: AgentMessage[]
  revision: number
}

export type TextContent = {
  type?: 'text'
  text: string
  textSignature?: string
}

export type ImageContent = {
  type: 'image'
  data?: string
  mimeType?: string
  url?: string
}

export type AssistantContentBlock =
  | { type: 'text'; text: string; textSignature?: string }
  | {
      type: 'thinking'
      thinking: string
      thinkingSignature?: string
      redacted?: boolean
    }
  | {
      type: 'toolCall'
      id: string
      name: string
      arguments: Record<string, unknown>
      thoughtSignature?: string
      namespace?: string
      argumentsError?: string
    }

export type AgentMessage =
  | {
      role: 'system'
      content: string | TextContent[]
      timestamp: number
    }
  | {
      role: 'user'
      content: string | Array<TextContent | ImageContent>
      timestamp: number
    }
  | {
      role: 'assistant'
      content: AssistantContentBlock[]
      api: string
      provider: string
      model: string
      responseModel?: string
      responseId?: string
      usage: unknown
      stopReason: string
      errorMessage?: string
      rawStopReason?: string
      timestamp: number
      durationMs?: number
    }
  | {
      role: 'toolResult'
      toolCallId: string
      toolName: string
      content: Array<TextContent | ImageContent>
      details?: unknown
      isError: boolean
      timestamp: number
    }

export type AssistantMessageEvent =
  | { type: 'start'; partial: AgentMessage }
  | { type: 'text_start'; content_index: number; partial: AgentMessage }
  | {
      type: 'text_delta'
      content_index: number
      delta: string
      partial: AgentMessage
    }
  | {
      type: 'text_end'
      content_index: number
      content: string
      partial: AgentMessage
    }
  | { type: 'thinking_start'; content_index: number; partial: AgentMessage }
  | {
      type: 'thinking_delta'
      content_index: number
      delta: string
      partial: AgentMessage
    }
  | {
      type: 'thinking_end'
      content_index: number
      content: string
      partial: AgentMessage
    }
  | { type: 'toolcall_start'; content_index: number; partial: AgentMessage }
  | {
      type: 'toolcall_delta'
      content_index: number
      delta: string
      partial: AgentMessage
    }
  | {
      type: 'toolcall_end'
      content_index: number
      tool_call: {
        id: string
        name: string
        arguments: Record<string, unknown>
      }
      partial: AgentMessage
    }
  | { type: 'done'; reason: string; message: AgentMessage }
  | { type: 'error'; reason: string; error: AgentMessage }

export type AgentToolResult = {
  content: TextContent[]
  details?: unknown
  structuredContent?: unknown
  usage?: unknown
  isError?: boolean
  terminate?: boolean
}

export type AgentEvent =
  | { type: 'agent_start' }
  | { type: 'agent_end'; messages: AgentMessage[] }
  | { type: 'turn_start' }
  | {
      type: 'turn_end'
      message: AgentMessage
      tool_results: AgentMessage[]
    }
  | { type: 'message_start'; message: AgentMessage }
  | {
      type: 'message_update'
      message: AgentMessage
      assistant_message_event: AssistantMessageEvent
    }
  | { type: 'message_end'; message: AgentMessage }
  | {
      type: 'tool_execution_start'
      tool_call_id: string
      tool_name: string
      args: unknown
    }
  | {
      type: 'tool_execution_update'
      tool_call_id: string
      tool_name: string
      args: unknown
      partial_result: AgentToolResult
    }
  | {
      type: 'tool_execution_end'
      tool_call_id: string
      tool_name: string
      result: AgentToolResult
      is_error: boolean
      duration_ms?: number
    }

export type WsClientMessage =
  | { type: 'auth'; token: string }
  | { type: 'prompt'; id?: string; sessionId: string; message: string }
  | { type: 'abort'; id?: string; sessionId: string }

export type WsServerMessage =
  | AgentEvent
  | {
      type: 'response'
      id?: string
      command: string
      success: boolean
      error?: string
    }
  | {
      type: 'session_meta'
      sessionId: string
      revision: number
      title?: string
      model?: string
    }
  | { type: 'error'; message: string }

export type ToolRunState = {
  toolCallId: string
  toolName: string
  args: unknown
  partialText: string
  resultText?: string
  isError?: boolean
  status: 'running' | 'done' | 'error'
  durationMs?: number
}

export type TranscriptItem =
  | { kind: 'user'; id: string; text: string; timestamp: number }
  | {
      kind: 'assistant'
      id: string
      message: Extract<AgentMessage, { role: 'assistant' }>
      streaming: boolean
    }
  | {
      kind: 'tool'
      id: string
      run: ToolRunState
    }
  | { kind: 'system'; id: string; text: string; timestamp: number }

export type TranscriptState = {
  items: TranscriptItem[]
  busy: boolean
  revision: number
}
