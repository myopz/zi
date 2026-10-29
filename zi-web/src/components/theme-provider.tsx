import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'
import { getPref, setPref } from '@/lib/idb/sync'
import {
  applyTheme,
  isThemePreference,
  readStoredTheme,
  THEME_PREF_KEY,
  type ResolvedTheme,
  type ThemePreference,
  writeStoredTheme,
} from '@/lib/theme'

type ThemeContextValue = {
  theme: ThemePreference
  resolved: ResolvedTheme
  setTheme: (theme: ThemePreference) => void
}

const ThemeContext = createContext<ThemeContextValue | null>(null)

export function ThemeProvider({
  userId,
  children,
}: {
  userId?: string | null
  children: ReactNode
}) {
  const [theme, setThemeState] = useState<ThemePreference>(() =>
    readStoredTheme(),
  )
  const [resolved, setResolved] = useState<ResolvedTheme>(() =>
    applyTheme(readStoredTheme()),
  )

  useEffect(() => {
    setResolved(applyTheme(theme))
    writeStoredTheme(theme)

    if (theme !== 'system') {
      return
    }
    const mq = window.matchMedia('(prefers-color-scheme: dark)')
    const onChange = () => setResolved(applyTheme('system'))
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [theme])

  useEffect(() => {
    if (!userId) {
      return
    }
    let cancelled = false
    void (async () => {
      const stored = await getPref<unknown>(userId, THEME_PREF_KEY, null)
      if (cancelled) {
        return
      }
      if (isThemePreference(stored)) {
        setThemeState(stored)
        writeStoredTheme(stored)
        return
      }
      await setPref(userId, THEME_PREF_KEY, readStoredTheme())
    })()
    return () => {
      cancelled = true
    }
  }, [userId])

  const setTheme = useCallback(
    (next: ThemePreference) => {
      setThemeState(next)
      writeStoredTheme(next)
      if (userId) {
        void setPref(userId, THEME_PREF_KEY, next)
      }
    },
    [userId],
  )

  const value = useMemo(
    () => ({ theme, resolved, setTheme }),
    [theme, resolved, setTheme],
  )

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  )
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext)
  if (!ctx) {
    throw new Error('useTheme must be used within ThemeProvider')
  }
  return ctx
}
