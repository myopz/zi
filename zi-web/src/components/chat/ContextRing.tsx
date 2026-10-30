import type { ContextUsageSnapshot } from '@/lib/types'
import { formatTokens } from '@/lib/usage'
import { cn } from '@/lib/utils'

/** Cursor-style circular context fill vs the effective auto-compact limit. */
export function ContextRing({
  usage,
  className,
}: {
  usage: ContextUsageSnapshot | undefined
  className?: string
}) {
  const denom =
    usage && usage.compactLimit > 0
      ? usage.compactLimit
      : usage && usage.contextWindow > 0
        ? usage.contextWindow
        : 0

  if (!usage || denom <= 0) {
    return (
      <div
        className={cn(
          'flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-dashed border-border text-[10px] text-muted-foreground',
          className,
        )}
        title="Context usage updates after the next model turn"
        aria-label="Context usage unknown"
      >
        —
      </div>
    )
  }

  const ratio = Math.min(1, usage.estimatedTokens / denom)
  const percent = Math.round(ratio * 100)
  const size = 36
  const stroke = 3
  const r = (size - stroke) / 2
  const c = 2 * Math.PI * r
  const filled = c * ratio
  const tone =
    usage.nearLimit || percent >= 90
      ? 'text-amber-500'
      : percent >= 70
        ? 'text-yellow-500'
        : 'text-emerald-500'

  const tip = [
    `${formatTokens(usage.estimatedTokens)} / ${formatTokens(denom)} compact limit (${percent}%)`,
    usage.contextWindow > 0
      ? `model window ${formatTokens(usage.contextWindow)}`
      : null,
    usage.foldedToolResults > 0
      ? `${usage.foldedToolResults} older tool result(s) folded`
      : null,
    `Σ in ${formatTokens(usage.cumulativeInput)} · out ${formatTokens(usage.cumulativeOutput)}`,
    usage.nearLimit ? 'Near auto-compact threshold' : null,
  ]
    .filter(Boolean)
    .join('\n')

  return (
    <button
      type="button"
      className={cn(
        'relative flex h-9 w-9 shrink-0 items-center justify-center rounded-full',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        className,
      )}
      title={tip}
      aria-label={`Context ${percent}% of compact limit`}
    >
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        className="-rotate-90"
        aria-hidden
      >
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="currentColor"
          strokeWidth={stroke}
          className="text-muted/40"
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="currentColor"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${filled} ${c - filled}`}
          className={cn('transition-[stroke-dasharray] duration-300', tone)}
        />
      </svg>
      <span
        className={cn(
          'pointer-events-none absolute inset-0 flex items-center justify-center text-[10px] font-medium tabular-nums',
          tone,
        )}
      >
        {percent}
      </span>
    </button>
  )
}
