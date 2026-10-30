import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import type { TranscriptItem } from '@/lib/types'
import { formatTokens, parseAssistantUsage, sumTranscriptUsage } from './usage.ts'

describe('usage helpers', () => {
  it('formats token counts', () => {
    assert.equal(formatTokens(340), '340')
    assert.equal(formatTokens(12_400), '12k')
    assert.equal(formatTokens(1_250_000), '1.3M')
  })

  it('parses camelCase usage', () => {
    const u = parseAssistantUsage({
      input: 18053,
      output: 75,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 18128,
    })
    assert.equal(u?.input, 18053)
    assert.equal(u?.totalTokens, 18128)
  })

  it('sums assistant turns and skips empty/streaming', () => {
    const items: TranscriptItem[] = [
      {
        kind: 'assistant',
        id: 'a1',
        streaming: false,
        message: {
          role: 'assistant',
          content: [{ type: 'text', text: 'hi' }],
          api: 'x',
          provider: 'x',
          model: 'm',
          usage: { input: 1000, output: 10, cacheRead: 0, cacheWrite: 0, totalTokens: 1010 },
          stopReason: 'stop',
          timestamp: 1,
        },
      },
      {
        kind: 'assistant',
        id: 'a2',
        streaming: true,
        message: {
          role: 'assistant',
          content: [{ type: 'text', text: '' }],
          api: 'x',
          provider: 'x',
          model: 'm',
          usage: { input: 9999, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 9999 },
          stopReason: 'stop',
          timestamp: 2,
        },
      },
      {
        kind: 'assistant',
        id: 'a3',
        streaming: false,
        message: {
          role: 'assistant',
          content: [{ type: 'text', text: 'ok' }],
          api: 'x',
          provider: 'x',
          model: 'm',
          usage: { input: 2000, output: 20, cacheRead: 100, cacheWrite: 0, totalTokens: 2120 },
          stopReason: 'stop',
          timestamp: 3,
        },
      },
    ]
    const t = sumTranscriptUsage(items)
    assert.equal(t.turns, 2)
    assert.equal(t.input, 3000)
    assert.equal(t.output, 30)
    assert.equal(t.cacheRead, 100)
    assert.equal(t.totalTokens, 3130)
  })
})
