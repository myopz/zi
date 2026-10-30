import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { toolArgsSummary, toolStepLabel } from './tool-labels.ts'

describe('toolStepLabel', () => {
  it('maps coding-agent tools to Cursor-style labels', () => {
    assert.equal(toolStepLabel('read'), 'Read')
    assert.equal(toolStepLabel('grep'), 'Grepped')
    assert.equal(toolStepLabel('find'), 'Search files')
    assert.equal(toolStepLabel('ls'), 'Listed')
    assert.equal(toolStepLabel('explore'), 'Explored')
    assert.equal(toolStepLabel('bash'), 'Ran command')
    assert.equal(toolStepLabel('edit'), 'Edited')
    assert.equal(toolStepLabel('write'), 'Wrote')
  })

  it('falls back to the raw name for unknown tools', () => {
    assert.equal(toolStepLabel('custom_tool'), 'custom_tool')
  })
})

describe('toolArgsSummary', () => {
  it('extracts path / pattern / command / goal', () => {
    assert.equal(toolArgsSummary('read', { path: 'src/a.ts' }), 'src/a.ts')
    assert.equal(toolArgsSummary('grep', { pattern: 'foo' }), 'foo')
    assert.equal(toolArgsSummary('find', { pattern: '**/*.rs' }), '**/*.rs')
    assert.equal(toolArgsSummary('bash', { command: 'cargo test' }), 'cargo test')
    assert.equal(toolArgsSummary('explore', { goal: 'how auth works' }), 'how auth works')
  })

  it('returns null when nothing useful is present', () => {
    assert.equal(toolArgsSummary('read', {}), null)
    assert.equal(toolArgsSummary('grep', null), null)
  })
})
