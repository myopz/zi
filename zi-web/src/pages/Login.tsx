import { useEffect, useState, type FormEvent } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  consumeTokenFromHash,
  loginWithToken,
  type AuthState,
} from '@/lib/auth'

export function Login({ onSuccess }: { onSuccess: (auth: AuthState) => void }) {
  const [token, setToken] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    const fromHash = consumeTokenFromHash()
    if (!fromHash) {
      return
    }
    setToken(fromHash)
    void (async () => {
      setLoading(true)
      setError(null)
      try {
        const auth = await loginWithToken(fromHash)
        onSuccess(auth)
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setLoading(false)
      }
    })()
  }, [onSuccess])

  async function submit(e: FormEvent) {
    e.preventDefault()
    const value = token.trim()
    if (!value) {
      setError('Paste a login token')
      return
    }
    setLoading(true)
    setError(null)
    try {
      const auth = await loginWithToken(value)
      onSuccess(auth)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex min-h-full items-center justify-center bg-background p-6">
      <form
        onSubmit={submit}
        className="w-full max-w-md rounded-xl border border-border bg-card p-6 shadow-sm"
      >
        <div className="mb-6">
          <div className="mb-2 font-mono text-xs tracking-[0.2em] text-primary uppercase">
            zi-web
          </div>
          <h1 className="text-xl font-semibold">Token login</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            Paste a seeded user token. Handoff URLs with{' '}
            <code className="text-foreground">#token=</code> are accepted and
            stripped immediately. Credentials stay in sessionStorage only.
          </p>
        </div>
        <label className="mb-2 block text-xs text-muted-foreground">
          Access token
        </label>
        <Input
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="paste token"
          disabled={loading}
        />
        {error ? (
          <p className="mt-3 text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}
        <Button type="submit" className="mt-4 w-full" disabled={loading}>
          {loading ? 'Signing in…' : 'Continue'}
        </Button>
      </form>
    </div>
  )
}
