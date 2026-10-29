import {
  cloneElement,
  isValidElement,
  useCallback,
  useId,
  useState,
  type CSSProperties,
  type FocusEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
} from 'react'
import { createPortal } from 'react-dom'
import { cn } from '@/lib/utils'

type TipChildProps = {
  onMouseEnter?: (e: MouseEvent) => void
  onMouseLeave?: (e: MouseEvent) => void
  onFocus?: (e: FocusEvent) => void
  onBlur?: (e: FocusEvent) => void
  'aria-describedby'?: string
  className?: string
  style?: CSSProperties
}

/**
 * Hover tip via portal. Handlers attach to the child element so tips work on
 * full-width buttons (wrapper-only hover is flaky with nested truncate).
 */
export function HoverTip({
  content,
  children,
  className,
}: {
  content: ReactNode
  children: ReactElement<TipChildProps>
  className?: string
  side?: 'right' | 'top' | 'bottom'
}) {
  const tipId = useId()
  const [open, setOpen] = useState(false)
  const [pos, setPos] = useState({ top: 0, left: 0 })

  const show = useCallback((el: HTMLElement) => {
    const rect = el.getBoundingClientRect()
    const width = Math.min(320, window.innerWidth - 24)
    let left = rect.right + 8
    if (left + width > window.innerWidth - 8) {
      left = Math.max(8, rect.left - width - 8)
    }
    setPos({
      top: Math.min(rect.top, window.innerHeight - 48),
      left,
    })
    setOpen(true)
  }, [])

  if (!isValidElement(children)) {
    return children
  }

  const child = cloneElement(children, {
    className: cn(children.props.className, className),
    'aria-describedby': open ? tipId : undefined,
    onMouseEnter: (e: MouseEvent) => {
      children.props.onMouseEnter?.(e)
      show(e.currentTarget as HTMLElement)
    },
    onMouseLeave: (e: MouseEvent) => {
      children.props.onMouseLeave?.(e)
      setOpen(false)
    },
    onFocus: (e: FocusEvent) => {
      children.props.onFocus?.(e)
      show(e.currentTarget as HTMLElement)
    },
    onBlur: (e: FocusEvent) => {
      children.props.onBlur?.(e)
      setOpen(false)
    },
  })

  return (
    <>
      {child}
      {open
        ? createPortal(
            <div
              id={tipId}
              role="tooltip"
              className="pointer-events-none fixed z-[300] max-w-[min(20rem,calc(100vw-2rem))] whitespace-pre-wrap break-all rounded-md border border-border bg-popover px-2.5 py-1.5 text-left text-[11px] leading-snug text-popover-foreground shadow-lg"
              style={{ top: pos.top, left: pos.left }}
            >
              {content}
            </div>,
            document.body,
          )
        : null}
    </>
  )
}
