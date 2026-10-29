import { type ClassValue, clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/** Mask ids/tokens for display: keep a short prefix/suffix, middle as `***`. */
export function maskSecret(value: string, visible = 4): string {
  const trimmed = value.trim()
  if (trimmed.length <= visible * 2) {
    return '*'.repeat(Math.max(trimmed.length, 8))
  }
  return `${trimmed.slice(0, visible)}${'*'.repeat(8)}${trimmed.slice(-visible)}`
}

/** Compact path for sidebar rows; full string still belongs in `title`. */
export function shortenPath(path: string, max = 36): string {
  const trimmed = path.trim()
  if (!trimmed) {
    return trimmed
  }
  let display = trimmed.replace(/^\/Users\/[^/]+/, '~')
  display = display.replace(/^\/home\/[^/]+/, '~')
  if (display.length <= max) {
    return display
  }
  const head = Math.max(8, Math.floor(max / 2) - 1)
  const tail = max - head - 1
  return `${display.slice(0, head)}…${display.slice(-tail)}`
}

const UUID_LEAF = /^[0-9a-f]{8}-[0-9a-f-]{27,}$/i

/** Suggest a cwd under the user default sandbox that is not already used. */
export function suggestNewWorkspaceCwd(
  defaultCwd: string,
  existingCwds: string[],
  name?: string,
): string {
  const base = defaultCwd.trim().replace(/\/+$/, '')
  if (!base) {
    return ''
  }
  const raw = (name?.trim() || 'project').toLowerCase()
  const slug =
    raw
      .replace(/[^a-z0-9_-]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 40) || 'project'
  const taken = new Set(existingCwds.map((c) => c.replace(/\/+$/, '')))
  let candidate = `${base}/${slug}`
  let n = 2
  while (taken.has(candidate)) {
    candidate = `${base}/${slug}-${n}`
    n += 1
  }
  return candidate
}

/** Prefer a human label over a uuid-looking folder name. */
export function workspaceLabel(name: string, cwd: string): string {
  const label = name.trim()
  if (label && !UUID_LEAF.test(label)) {
    return label
  }
  const leaf = cwd.split('/').filter(Boolean).pop() ?? ''
  if (leaf && !UUID_LEAF.test(leaf)) {
    return leaf
  }
  return 'Default'
}
