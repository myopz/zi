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

/** True when the stored title is still a generic / auto placeholder. */
export function isPlaceholderSessionTitle(title: string | undefined | null): boolean {
  const t = (title ?? '').trim()
  if (!t) {
    return true
  }
  if (/^(new session|untitled|session)$/i.test(t)) {
    return true
  }
  // API defaults before first prompt: "Chat 01a18a33" / "Session 08:24"
  if (/^chat\s+[0-9a-f-]{6,}$/i.test(t)) {
    return true
  }
  if (/^session\s+\d{1,2}:\d{2}$/i.test(t)) {
    return true
  }
  return false
}

/** Sidebar label: prefer real title; fall back to time so rows are distinguishable. */
export function sessionDisplayTitle(session: {
  title: string
  updatedAt?: string
  createdAt?: string
}): string {
  const raw = session.title?.trim() || ''
  if (raw && !isPlaceholderSessionTitle(raw)) {
    return raw
  }
  // Short id / timed defaults are already unique enough to show as-is.
  if (/^chat\s+[0-9a-f-]{6,}$/i.test(raw) || /^session\s+\d{1,2}:\d{2}$/i.test(raw)) {
    return raw
  }
  const when = formatSessionWhen(session.updatedAt || session.createdAt || '')
  return when ? `Chat · ${when}` : 'Chat'
}

/** Parse API timestamps: ISO-8601 or zi-web `{unixSecs}.{millis}Z`. */
export function parseSessionStamp(raw: string): Date | null {
  const trimmed = raw.trim()
  if (!trimmed) {
    return null
  }
  const custom = /^(\d+)(?:\.(\d+))?Z?$/.exec(trimmed)
  if (custom) {
    const secs = Number(custom[1])
    const frac = custom[2] ?? '0'
    const millis = Number(frac.padEnd(3, '0').slice(0, 3))
    const d = new Date(secs * 1000 + millis)
    return Number.isNaN(d.getTime()) ? null : d
  }
  const d = new Date(trimmed)
  return Number.isNaN(d.getTime()) ? null : d
}

/** Compact local time / relative day for session rows. */
export function formatSessionWhen(iso: string): string {
  const d = parseSessionStamp(iso)
  if (!d) {
    return ''
  }
  const now = new Date()
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  const hh = String(d.getHours()).padStart(2, '0')
  const mm = String(d.getMinutes()).padStart(2, '0')
  if (sameDay) {
    return `${hh}:${mm}`
  }
  const yesterday = new Date(now)
  yesterday.setDate(now.getDate() - 1)
  const isYesterday =
    d.getFullYear() === yesterday.getFullYear() &&
    d.getMonth() === yesterday.getMonth() &&
    d.getDate() === yesterday.getDate()
  if (isYesterday) {
    return `昨天 ${hh}:${mm}`
  }
  const mon = String(d.getMonth() + 1).padStart(2, '0')
  const day = String(d.getDate()).padStart(2, '0')
  if (d.getFullYear() === now.getFullYear()) {
    return `${mon}-${day} ${hh}:${mm}`
  }
  return `${d.getFullYear()}-${mon}-${day}`
}
