import type { SlashCommand } from '@/lib/types'

/** Builtin fallback when `/api/slash-commands` is unavailable. */
export const FALLBACK_SLASH_COMMANDS: SlashCommand[] = [
  {
    name: 'model',
    summary: 'List or switch model (`/model` or `/model <id>`)',
  },
  {
    name: 'models',
    summary: 'alias for /model',
  },
  {
    name: 'hotkeys',
    summary: 'Show keyboard shortcuts and slash commands',
  },
  {
    name: 'help',
    summary: 'alias for /hotkeys',
  },
  {
    name: 'copy',
    summary: 'Copy last assistant reply to the clipboard',
  },
  {
    name: 'new',
    summary: 'Start a new session (clears transcript)',
  },
  {
    name: 'clear',
    summary: 'Clear conversation (keeps session file)',
  },
  {
    name: 'session',
    summary: 'Show current session path and stats',
  },
  {
    name: 'compact',
    summary:
      'Compact context now (also /summarize; auto at min(90% window, 48k))',
  },
  {
    name: 'summarize',
    summary: 'alias for /compact',
  },
  {
    name: 'tree',
    summary: 'Show session entry tree',
  },
  {
    name: 'fork',
    summary: 'Fork the current session into a new branch',
  },
  {
    name: 'mcp',
    summary: 'Show MCP server status',
  },
  {
    name: 'trust',
    summary: 'Trust or untrust this project for `.zi` resources',
  },
  {
    name: 'reload',
    summary: 'Reload prompts/skills and host packs',
  },
  {
    name: 'extensions',
    summary: 'List discovered host/WASM extensions and pack status',
  },
]

/** Active `/query` at the start of the draft (no space yet). */
export function parseSlashQuery(text: string): string | null {
  const match = /^\/([^\s]*)$/.exec(text)
  if (!match) {
    return null
  }
  return match[1] ?? ''
}

/** Cursor-style substring filter on command name. */
export function filterSlashCommands(
  commands: SlashCommand[],
  query: string,
): SlashCommand[] {
  const q = query.trim().toLowerCase()
  if (!q) {
    return commands
  }
  return commands.filter((cmd) => cmd.name.toLowerCase().includes(q))
}
