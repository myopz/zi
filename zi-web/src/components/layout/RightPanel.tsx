import { PanelRightClose, X } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { useTheme } from '@/components/theme-provider'
import { THEME_OPTIONS, type ThemePreference } from '@/lib/theme'
import type { SessionStatus } from '@/lib/types'
import { cn } from '@/lib/utils'

export function RightPanel({
  status,
  revision,
  settingsOpen,
  username,
  mobile,
  onCollapse,
  onCloseSettings,
}: {
  status: SessionStatus | null
  revision: number
  settingsOpen: boolean
  username: string
  /** Drawer mode: fixed overlay on small screens. */
  mobile?: boolean
  onCollapse?: () => void
  onCloseSettings: () => void
}) {
  const { theme, setTheme } = useTheme()

  return (
    <aside
      className={cn(
        'flex h-full w-72 shrink-0 flex-col border-l border-border bg-sidebar',
        mobile
          ? 'fixed inset-y-0 right-0 z-40 max-w-[85vw] shadow-xl'
          : 'relative',
      )}
    >
      <div className="flex items-center gap-2 px-3 py-3">
        <div className="min-w-0 flex-1 text-sm font-medium">
          {settingsOpen ? 'Settings' : 'Context'}
        </div>
        {onCollapse ? (
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="h-8 w-8 shrink-0"
            onClick={onCollapse}
            title={mobile ? 'Close panel' : 'Collapse panel'}
            aria-label={mobile ? 'Close panel' : 'Collapse panel'}
          >
            {mobile ? (
              <X className="h-4 w-4" />
            ) : (
              <PanelRightClose className="h-4 w-4" />
            )}
          </Button>
        ) : null}
      </div>
      <div className="flex-1 space-y-3 overflow-auto px-3 pb-3 text-sm">
        {settingsOpen ? (
          <div className="space-y-3">
            <div className="rounded-md border border-border bg-card p-3">
              <div className="text-xs text-muted-foreground">Signed in as</div>
              <div className="mt-1 font-medium">{username}</div>
            </div>
            <div className="rounded-md border border-border bg-card p-3">
              <div className="mb-2 text-xs text-muted-foreground">Theme</div>
              <Select
                value={theme}
                onValueChange={(value) => setTheme(value as ThemePreference)}
              >
                <SelectTrigger aria-label="Theme">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {THEME_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                Preference is stored in this browser (IndexedDB + localStorage).
                Default is light.
              </p>
            </div>
            <div className="rounded-md border border-border bg-card p-3 text-xs leading-relaxed text-muted-foreground">
              Auth uses a long-lived login token (OpenClaw-style). Paste a token
              on the login page or open{' '}
              <code className="text-foreground">#token=&lt;raw&gt;</code>. The
              access token stays in <code className="text-foreground">sessionStorage</code>{' '}
              only for this tab. Logout clears it and the IndexedDB cache for
              this user. Raw tokens are never shown again after login.
            </div>
            <button
              type="button"
              className="text-xs text-primary hover:underline"
              onClick={onCloseSettings}
            >
              Close settings
            </button>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="rounded-md border border-border bg-card p-3">
              <div className="mb-2 text-xs text-muted-foreground">Status</div>
              <div className="flex flex-wrap gap-2">
                <Badge variant={status?.busy ? 'default' : 'secondary'}>
                  {status?.busy ? 'busy' : 'idle'}
                </Badge>
                {status?.model ? (
                  <Badge variant="outline" className="font-mono">
                    {status.model}
                  </Badge>
                ) : null}
                <Badge variant="outline">rev {revision}</Badge>
              </div>
              {status?.error ? (
                <p className="mt-2 text-xs text-destructive">{status.error}</p>
              ) : null}
            </div>
            <div className="rounded-md border border-dashed border-border px-3 py-6 text-center text-xs text-muted-foreground">
              Right pane placeholder (files / tools / diffs later).
            </div>
          </div>
        )}
      </div>
    </aside>
  )
}
