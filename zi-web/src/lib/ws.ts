import type { AgentEvent, WsClientMessage, WsServerMessage } from '@/lib/types'

export type AgentSocketHandlers = {
  onEvent: (event: AgentEvent) => void
  onResponse?: (msg: Extract<WsServerMessage, { type: 'response' }>) => void
  onSessionMeta?: (
    msg: Extract<WsServerMessage, { type: 'session_meta' }>,
  ) => void
  onError?: (message: string) => void
  onConnectionChange?: (connected: boolean) => void
}

function isAgentEvent(msg: WsServerMessage): msg is AgentEvent {
  return (
    msg.type === 'agent_start' ||
    msg.type === 'agent_end' ||
    msg.type === 'turn_start' ||
    msg.type === 'turn_end' ||
    msg.type === 'message_start' ||
    msg.type === 'message_update' ||
    msg.type === 'message_end' ||
    msg.type === 'tool_execution_start' ||
    msg.type === 'tool_execution_update' ||
    msg.type === 'tool_execution_end'
  )
}

export class AgentSocket {
  private ws: WebSocket | null = null
  private readonly token: string
  private readonly handlers: AgentSocketHandlers
  private closed = false
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private attempt = 0
  /** True after server accepts `{type:"auth"}`. */
  private authed = false
  /** Outbound frames waiting for OPEN + auth (prompts must not be dropped). */
  private readonly pending: WsClientMessage[] = []

  constructor(token: string, handlers: AgentSocketHandlers) {
    this.token = token
    this.handlers = handlers
    this.connect()
  }

  private wsUrl(): string {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    return `${proto}//${window.location.host}/ws`
  }

  private connect(): void {
    if (this.closed) {
      return
    }
    this.authed = false
    const ws = new WebSocket(this.wsUrl())
    this.ws = ws

    ws.onopen = () => {
      this.attempt = 0
      this.handlers.onConnectionChange?.(true)
      // Auth first; other queued frames flush after auth response.
      ws.send(JSON.stringify({ type: 'auth', token: this.token }))
    }

    ws.onmessage = (ev) => {
      let msg: WsServerMessage
      try {
        msg = JSON.parse(String(ev.data)) as WsServerMessage
      } catch {
        this.handlers.onError?.('Invalid WS JSON')
        return
      }
      if (isAgentEvent(msg)) {
        this.handlers.onEvent(msg)
        return
      }
      if (msg.type === 'response') {
        if (msg.command === 'auth') {
          if (msg.success) {
            this.authed = true
            this.flushPending()
          } else {
            this.handlers.onError?.(msg.error || 'WebSocket auth failed')
          }
        }
        this.handlers.onResponse?.(msg)
        return
      }
      if (msg.type === 'session_meta') {
        this.handlers.onSessionMeta?.(msg)
        return
      }
      if (msg.type === 'error') {
        this.handlers.onError?.(msg.message)
      }
    }

    ws.onerror = () => {
      this.handlers.onError?.('WebSocket error')
    }

    ws.onclose = () => {
      this.authed = false
      this.handlers.onConnectionChange?.(false)
      this.ws = null
      if (!this.closed) {
        const delay = Math.min(10_000, 500 * 2 ** this.attempt)
        this.attempt += 1
        this.reconnectTimer = setTimeout(() => this.connect(), delay)
      }
    }
  }

  private flushPending(): void {
    if (!this.authed || this.ws?.readyState !== WebSocket.OPEN) {
      return
    }
    while (this.pending.length > 0) {
      const next = this.pending.shift()
      if (!next) {
        break
      }
      this.ws.send(JSON.stringify(next))
    }
  }

  send(msg: WsClientMessage): void {
    if (this.closed) {
      return
    }
    // Auth is sent from onopen only.
    if (msg.type === 'auth') {
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.ws.send(JSON.stringify(msg))
      }
      return
    }
    if (this.authed && this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg))
      return
    }
    this.pending.push(msg)
  }

  prompt(sessionId: string, message: string, id?: string): void {
    this.send({ type: 'prompt', sessionId, message, id })
  }

  abort(sessionId: string, id?: string): void {
    this.send({ type: 'abort', sessionId, id })
  }

  close(): void {
    this.closed = true
    this.pending.length = 0
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.ws?.close()
    this.ws = null
  }
}
