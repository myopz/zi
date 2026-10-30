import type { AssistantUsage, TranscriptItem } from '@/lib/types'

/** Format token counts for UI (1.2M / 12k / 340). */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n < 0) {
    return '0'
  }
  if (n >= 1_000_000) {
    return `${(n / 1_000_000).toFixed(1)}M`
  }
  if (n >= 1_000) {
    return `${(n / 1_000).toFixed(n >= 10_000 ? 0 : 1)}k`
  }
  return String(Math.round(n))
}

/** Parse camelCase Usage from assistant messages / WS payloads. */
export function parseAssistantUsage(raw: unknown): AssistantUsage | null {
  if (!raw || typeof raw !== 'object') {
    return null
  }
  const o = raw as Record<string, unknown>
  const input = Number(o.input ?? 0)
  const output = Number(o.output ?? 0)
  const cacheRead = Number(o.cacheRead ?? o.cache_read ?? 0)
  const cacheWrite = Number(o.cacheWrite ?? o.cache_write ?? 0)
  const totalTokens = Number(
    o.totalTokens ?? o.total_tokens ?? input + output + cacheRead + cacheWrite,
  )
  if (![input, output, cacheRead, cacheWrite, totalTokens].every(Number.isFinite)) {
    return null
  }
  return {
    input,
    output,
    cacheRead,
    cacheWrite,
    totalTokens,
  }
}

export type SessionTokenTotals = {
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
  totalTokens: number
  turns: number
}

/** Sum usage across finished assistant turns in the transcript. */
export function sumTranscriptUsage(items: TranscriptItem[]): SessionTokenTotals {
  let input = 0
  let output = 0
  let cacheRead = 0
  let cacheWrite = 0
  let totalTokens = 0
  let turns = 0
  for (const item of items) {
    if (item.kind !== 'assistant' || item.streaming) {
      continue
    }
    const u = parseAssistantUsage(item.message.usage)
    if (!u) {
      continue
    }
    if (u.input === 0 && u.output === 0 && u.totalTokens === 0) {
      continue
    }
    turns += 1
    input += u.input
    output += u.output
    cacheRead += u.cacheRead
    cacheWrite += u.cacheWrite
    totalTokens += u.totalTokens || u.input + u.output
  }
  return { input, output, cacheRead, cacheWrite, totalTokens, turns }
}
