/** Cursor-style step labels for coding-agent tools in the Web transcript. */

const LABELS: Record<string, string> = {
  read: 'Read',
  grep: 'Grepped',
  find: 'Search files',
  ls: 'Listed',
  explore: 'Explored',
  bash: 'Ran command',
  edit: 'Edited',
  write: 'Wrote',
}

/** Display label for a tool name (falls back to the raw name). */
export function toolStepLabel(toolName: string): string {
  return LABELS[toolName] ?? toolName
}

function asRecord(args: unknown): Record<string, unknown> | null {
  if (args == null || typeof args !== 'object' || Array.isArray(args)) {
    return null
  }
  return args as Record<string, unknown>
}

function pickString(obj: Record<string, unknown>, key: string): string | null {
  const v = obj[key]
  return typeof v === 'string' && v.length > 0 ? v : null
}

/** One-line summary from tool args (path / pattern / command / goal). */
export function toolArgsSummary(toolName: string, args: unknown): string | null {
  const obj = asRecord(args)
  if (!obj) {
    return null
  }
  switch (toolName) {
    case 'read':
    case 'ls':
    case 'edit':
    case 'write':
      return pickString(obj, 'path')
    case 'grep':
      return (
        pickString(obj, 'pattern') ??
        pickString(obj, 'path') ??
        null
      )
    case 'find':
      return pickString(obj, 'pattern')
    case 'bash':
      return pickString(obj, 'command')
    case 'explore':
      return pickString(obj, 'goal')
    default: {
      return (
        pickString(obj, 'path') ??
        pickString(obj, 'pattern') ??
        pickString(obj, 'command') ??
        pickString(obj, 'goal') ??
        null
      )
    }
  }
}
