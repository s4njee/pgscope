import { useEffect, useLayoutEffect, useRef, useState } from 'react'

export interface MenuItem {
  label: string
  onSelect: () => void | Promise<void>
  /** Right-aligned hint, e.g. a shortcut or a preview of the value. */
  hint?: string
  disabled?: boolean
  /** Renders a divider above this item. */
  separatorBefore?: boolean
}

interface Props {
  x: number
  y: number
  items: MenuItem[]
  onClose: () => void
}

/**
 * Keep the menu inside the window rather than letting it run off an edge.
 *
 * @param x - `number` — the requested left edge, in viewport pixels.
 * @param y - `number` — the requested top edge, in viewport pixels.
 * @param el - `HTMLElement | null` — the rendered menu, measured for its size; `null`
 *   before the first layout, in which case the coordinates pass through unchanged.
 * @returns `{ left: number; top: number }` — viewport pixel coordinates, pulled inside
 *   an 8px margin and flipped above the cursor when there is no room below.
 */
function clampToViewport(x: number, y: number, el: HTMLElement | null) {
  if (!el) return { left: x, top: y }
  const { offsetWidth: w, offsetHeight: h } = el
  const margin = 8
  return {
    left: Math.min(x, window.innerWidth - w - margin),
    // Flip above the cursor when there isn't room below.
    top: y + h + margin > window.innerHeight ? Math.max(margin, y - h) : y,
  }
}

/**
 * A menu floating at viewport coordinates `x`/`y` — where the right-click
 * happened, not a position within any parent.
 *
 * Listeners are attached to `window` in the capture phase so the menu closes
 * before whatever was clicked underneath reacts to it, and Escape here never
 * reaches a modal that would also close on it.
 *
 * @param props - `{ x: number; y: number; items: MenuItem[]; onClose: () => void }`
 *   - `x` — viewport x of the click that opened the menu, before clamping.
 *   - `y` — viewport y of that click, before clamping.
 *   - `items` — the entries in render order; `disabled` ones are skipped by the arrow
 *     keys, and `separatorBefore` draws a divider above the entry.
 *   - `onClose` — dismisses the menu; called after any selection and on every
 *     outside click, Escape, scroll, or window blur.
 * @returns `JSX.Element` — the floating menu, absolutely positioned at the clamped
 *   coordinates.
 */
export function ContextMenu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })
  const [active, setActive] = useState(0)

  useLayoutEffect(() => {
    setPos(clampToViewport(x, y, ref.current))
  }, [x, y])

  // Dismiss on outside click, Escape, scroll, or window blur — a menu left
  // floating over stale rows is worse than no menu.
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onClose()
        return
      }
      const enabled = items.filter((i) => !i.disabled)
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        const delta = e.key === 'ArrowDown' ? 1 : -1
        setActive((a) => {
          let next = a
          for (let step = 0; step < items.length; step++) {
            next = (next + delta + items.length) % items.length
            if (!items[next].disabled) break
          }
          return next
        })
      } else if (e.key === 'Enter' && enabled.length > 0) {
        e.preventDefault()
        const item = items[active]
        if (item && !item.disabled) {
          void item.onSelect()
          onClose()
        }
      }
    }

    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('blur', onClose)
    window.addEventListener('scroll', onClose, true)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('blur', onClose)
      window.removeEventListener('scroll', onClose, true)
    }
  }, [items, active, onClose])

  return (
    <div className="ctx" ref={ref} style={{ left: pos.left, top: pos.top }} role="menu">
      {items.map((item, i) => (
        <div key={`${item.label}-${i}`} style={{ display: 'contents' }}>
          {item.separatorBefore && <div className="ctx__sep" />}
          <button
            className={`ctx__item${i === active ? ' ctx__item--active' : ''}`}
            disabled={item.disabled}
            role="menuitem"
            onMouseEnter={() => setActive(i)}
            onClick={() => {
              void item.onSelect()
              onClose()
            }}
          >
            <span className="ctx__label">{item.label}</span>
            {item.hint && <span className="ctx__hint">{item.hint}</span>}
          </button>
        </div>
      ))}
    </div>
  )
}
