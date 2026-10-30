import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  appendOptimisticUser,
  applySlashResult,
  collapseIdenticalToolResults,
  createTranscriptState,
  messagesToTranscript,
  reduceAgentEvent,
} from './reducer.ts'

const ARCH_EDIT =
  'Updated /Users/zhangjianlong/workspace/forkmess/pi/ARCHITECTURE.md (1 edit(s))'

describe('collapseIdenticalToolResults', () => {
  it('merges consecutive identical edit results into one card', () => {
    const items = messagesToTranscript([
      {
        role: 'toolResult',
        toolCallId: 'a',
        toolName: 'edit',
        content: [{ text: 'Updated /tmp/ARCHITECTURE.md (1 edit(s))' }],
        isError: false,
        timestamp: 1,
      },
      {
        role: 'toolResult',
        toolCallId: 'b',
        toolName: 'edit',
        content: [{ text: 'Updated /tmp/ARCHITECTURE.md (1 edit(s))' }],
        isError: false,
        timestamp: 2,
      },
      {
        role: 'toolResult',
        toolCallId: 'c',
        toolName: 'edit',
        content: [{ text: 'Updated /tmp/ARCHITECTURE.md (1 edit(s))' }],
        isError: false,
        timestamp: 3,
      },
    ])
    assert.equal(items.length, 1)
    assert.equal(items[0]?.kind, 'tool')
    if (items[0]?.kind === 'tool') {
      assert.equal(items[0].run.repeatCount, 3)
      assert.equal(items[0].run.resultText, 'Updated /tmp/ARCHITECTURE.md (1 edit(s))')
    }
  })

  it('collapses eleven identical ARCHITECTURE.md edit results from the bug session', () => {
    const messages = Array.from({ length: 11 }, (_, i) => ({
      role: 'toolResult' as const,
      toolCallId: `call_edit_${i}`,
      toolName: 'edit',
      content: [{ text: ARCH_EDIT }],
      isError: false,
      timestamp: i + 1,
    }))
    const items = messagesToTranscript(messages)
    assert.equal(items.length, 1, 'UI must show one RESULT card, not eleven')
    if (items[0]?.kind === 'tool') {
      assert.equal(items[0].run.repeatCount, 11)
      assert.equal(items[0].run.resultText, ARCH_EDIT)
    }
  })

  it('merges live tool_execution_end events with the same result text', () => {
    let state = createTranscriptState()
    for (let i = 0; i < 5; i++) {
      state = reduceAgentEvent(state, {
        type: 'tool_execution_start',
        tool_call_id: `live-${i}`,
        tool_name: 'edit',
        args: { path: 'ARCHITECTURE.md' },
      })
      state = reduceAgentEvent(state, {
        type: 'tool_execution_end',
        tool_call_id: `live-${i}`,
        tool_name: 'edit',
        result: { content: [{ text: ARCH_EDIT }] },
        is_error: false,
        duration_ms: 10 + i,
      })
    }
    const tools = state.items.filter((item) => item.kind === 'tool')
    assert.equal(tools.length, 1, 'live stream must collapse identical edit ends')
    if (tools[0]?.kind === 'tool') {
      assert.equal(tools[0].run.repeatCount, 5)
      assert.equal(tools[0].run.status, 'done')
    }
  })

  it('merges message_end toolResult events the same way', () => {
    let state = createTranscriptState()
    for (let i = 0; i < 4; i++) {
      state = reduceAgentEvent(state, {
        type: 'message_end',
        message: {
          role: 'toolResult',
          toolCallId: `msg-${i}`,
          toolName: 'edit',
          content: [{ text: ARCH_EDIT }],
          isError: false,
          timestamp: 100 + i,
        },
      })
    }
    const tools = state.items.filter((item) => item.kind === 'tool')
    assert.equal(tools.length, 1)
    if (tools[0]?.kind === 'tool') {
      assert.equal(tools[0].run.repeatCount, 4)
    }
  })

  it('does not merge different result texts', () => {
    const collapsed = collapseIdenticalToolResults(
      messagesToTranscript([
        {
          role: 'toolResult',
          toolCallId: 'a',
          toolName: 'edit',
          content: [{ text: 'Updated /tmp/a.md (1 edit(s))' }],
          isError: false,
          timestamp: 1,
        },
        {
          role: 'toolResult',
          toolCallId: 'b',
          toolName: 'edit',
          content: [{ text: 'Updated /tmp/b.md (1 edit(s))' }],
          isError: false,
          timestamp: 2,
        },
      ]),
    )
    assert.equal(collapsed.length, 2)
  })

  it('does not merge across a different tool name', () => {
    const items = messagesToTranscript([
      {
        role: 'toolResult',
        toolCallId: 'a',
        toolName: 'edit',
        content: [{ text: ARCH_EDIT }],
        isError: false,
        timestamp: 1,
      },
      {
        role: 'toolResult',
        toolCallId: 'b',
        toolName: 'write',
        content: [{ text: ARCH_EDIT }],
        isError: false,
        timestamp: 2,
      },
    ])
    assert.equal(items.length, 2)
  })
})

describe('reduceAgentEvent agent_end', () => {
  it('keeps prior turns instead of replacing with the run delta', () => {
    let state = createTranscriptState([
      {
        role: 'user',
        content: 'first question',
        timestamp: 1,
      },
      {
        role: 'assistant',
        content: [{ type: 'text', text: 'first answer' }],
        api: 'openai-completions',
        provider: 'test',
        model: 'm',
        usage: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 0,
          cost: {
            input: 0,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            total: 0,
          },
        },
        stopReason: 'stop',
        timestamp: 2,
      },
    ])

    state = appendOptimisticUser(state, 'second question')
    state = reduceAgentEvent(state, {
      type: 'message_start',
      message: {
        role: 'assistant',
        content: [{ type: 'text', text: '' }],
        api: 'openai-completions',
        provider: 'test',
        model: 'm',
        usage: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 0,
          cost: {
            input: 0,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            total: 0,
          },
        },
        stopReason: 'stop',
        timestamp: 4,
      },
    })
    state = reduceAgentEvent(state, {
      type: 'message_end',
      message: {
        role: 'assistant',
        content: [{ type: 'text', text: 'second answer' }],
        api: 'openai-completions',
        provider: 'test',
        model: 'm',
        usage: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 0,
          cost: {
            input: 0,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            total: 0,
          },
        },
        stopReason: 'stop',
        timestamp: 4,
      },
    })

    // Server only sends this run's messages on agent_end.
    state = reduceAgentEvent(state, {
      type: 'agent_end',
      messages: [
        {
          role: 'user',
          content: 'second question',
          timestamp: 3,
        },
        {
          role: 'assistant',
          content: [{ type: 'text', text: 'second answer' }],
          api: 'openai-completions',
          provider: 'test',
          model: 'm',
          usage: {
            input: 0,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            totalTokens: 0,
            cost: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              total: 0,
            },
          },
          stopReason: 'stop',
          timestamp: 4,
        },
      ],
    })

    assert.equal(state.busy, false)
    assert.equal(state.items.length, 4)
    assert.equal(state.items[0]?.kind, 'user')
    if (state.items[0]?.kind === 'user') {
      assert.equal(state.items[0].text, 'first question')
    }
    assert.equal(state.items[3]?.kind, 'assistant')
    if (state.items[3]?.kind === 'assistant') {
      assert.equal(state.items[3].streaming, false)
    }
  })
})

describe('context_usage ring state', () => {
  it('stores context_usage for the composer ring', () => {
    let state = createTranscriptState()
    state = reduceAgentEvent(state, {
      type: 'context_usage',
      estimated_tokens: 72_000,
      context_window: 1_000_000,
      compact_limit: 48_000,
      folded_tool_results: 3,
      cumulative_input: 120_000,
      cumulative_output: 4_000,
      near_limit: false,
    })
    assert.equal(state.contextUsage?.estimatedTokens, 72_000)
    assert.equal(state.contextUsage?.compactLimit, 48_000)
    assert.equal(state.contextUsage?.nearLimit, false)
  })

  it('marks near_limit and updates after compaction', () => {
    let state = createTranscriptState()
    state = reduceAgentEvent(state, {
      type: 'context_usage',
      estimated_tokens: 48_000,
      context_window: 1_000_000,
      compact_limit: 48_000,
      folded_tool_results: 8,
      cumulative_input: 400_000,
      cumulative_output: 10_000,
      near_limit: true,
    })
    assert.equal(state.contextUsage?.nearLimit, true)
    state = reduceAgentEvent(state, {
      type: 'compaction',
      summary: 'Context compaction: omitted 12 earlier messages',
      tokens_before: 48_000,
      tokens_after: 22_000,
      dropped_messages: 12,
    })
    assert.equal(state.contextUsage?.estimatedTokens, 22_000)
    assert.equal(state.contextUsage?.nearLimit, false)
    assert.equal(state.contextUsage?.compactLimit, 48_000)
  })
})

describe('collapseIdenticalToolResults path merge', () => {
  it('merges consecutive ARCHITECTURE edits with different N edit(s)', () => {
    const path =
      '/Users/zhangjianlong/workspace/forkmess/pi/ARCHITECTURE.md'
    const items = messagesToTranscript([
      {
        role: 'toolResult',
        toolCallId: 'a',
        toolName: 'edit',
        content: [{ text: `Updated ${path} (2 edit(s))` }],
        isError: false,
        timestamp: 1,
      },
      {
        role: 'toolResult',
        toolCallId: 'b',
        toolName: 'edit',
        content: [{ text: `Updated ${path} (6 edit(s))` }],
        isError: false,
        timestamp: 2,
      },
      {
        role: 'toolResult',
        toolCallId: 'c',
        toolName: 'edit',
        content: [{ text: `Updated ${path} (5 edit(s))` }],
        isError: false,
        timestamp: 3,
      },
    ])
    assert.equal(items.length, 1)
    if (items[0]?.kind === 'tool') {
      assert.equal(items[0].run.repeatCount, 3)
      assert.equal(items[0].run.resultText, `Updated ${path} (5 edit(s))`)
    }
  })
})

describe('applySlashResult', () => {
  it('appends a system notice and clears busy', () => {
    let state = appendOptimisticUser(createTranscriptState(), '/models')
    state = applySlashResult(state, {
      kind: 'notice',
      text: 'Current: deepseek-flash\nAvailable:',
    })
    assert.equal(state.busy, false)
    assert.equal(state.items.length, 2)
    assert.equal(state.items[1]?.kind, 'system')
    if (state.items[1]?.kind === 'system') {
      assert.match(state.items[1].text, /Current:/)
    }
  })
})
