import type { LoginResponse, User } from '@/lib/types'

const ACCESS_TOKEN_KEY = 'zi.accessToken'
const USER_KEY = 'zi.user'

export type AuthState = {
  accessToken: string
  user: User
}

export function readAuth(): AuthState | null {
  const accessToken = sessionStorage.getItem(ACCESS_TOKEN_KEY)
  const rawUser = sessionStorage.getItem(USER_KEY)
  if (!accessToken || !rawUser) {
    return null
  }
  try {
    const user = JSON.parse(rawUser) as User
    if (!user?.id || !user?.username) {
      return null
    }
    return { accessToken, user }
  } catch {
    return null
  }
}

export function writeAuth(auth: AuthState): void {
  sessionStorage.setItem(ACCESS_TOKEN_KEY, auth.accessToken)
  sessionStorage.setItem(USER_KEY, JSON.stringify(auth.user))
}

export function clearAuth(): void {
  sessionStorage.removeItem(ACCESS_TOKEN_KEY)
  sessionStorage.removeItem(USER_KEY)
}

/** Strip `#token=` handoff from the URL hash (OpenClaw-style). */
export function consumeTokenFromHash(): string | null {
  const hash = window.location.hash
  if (!hash.startsWith('#token=')) {
    return null
  }
  const raw = decodeURIComponent(hash.slice('#token='.length))
  history.replaceState(null, '', `${window.location.pathname}${window.location.search}`)
  const token = raw.trim()
  return token.length > 0 ? token : null
}

export async function loginWithToken(token: string): Promise<AuthState> {
  const res = await fetch('/api/auth/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token }),
  })
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new Error(text || `Login failed (${res.status})`)
  }
  const body = (await res.json()) as LoginResponse
  if (!body.accessToken || !body.user?.id) {
    throw new Error('Invalid login response')
  }
  const auth = { accessToken: body.accessToken, user: body.user }
  writeAuth(auth)
  return auth
}

export async function logoutRemote(accessToken: string): Promise<void> {
  try {
    await fetch('/api/auth/logout', {
      method: 'POST',
      headers: { Authorization: `Bearer ${accessToken}` },
    })
  } catch {
    // Token-as-bearer may no-op; client still clears local state.
  }
}
