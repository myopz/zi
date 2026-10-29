import { useCallback, useEffect, useState } from 'react'
import { AppShell } from '@/components/layout/AppShell'
import { ThemeProvider } from '@/components/theme-provider'
import { Login } from '@/pages/Login'
import { clearAuth, readAuth, type AuthState } from '@/lib/auth'

export default function App() {
  const [auth, setAuth] = useState<AuthState | null>(() => readAuth())
  const [ready, setReady] = useState(false)

  useEffect(() => {
    setAuth(readAuth())
    setReady(true)
  }, [])

  const handleLogout = useCallback(() => {
    clearAuth()
    setAuth(null)
  }, [])

  if (!ready) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        Loading…
      </div>
    )
  }

  return (
    <ThemeProvider userId={auth?.user.id}>
      {auth ? (
        <AppShell auth={auth} onLogout={handleLogout} />
      ) : (
        <Login onSuccess={setAuth} />
      )}
    </ThemeProvider>
  )
}
