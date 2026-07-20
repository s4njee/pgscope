import { useRef, useState } from 'react'

import { ContextMenu, type MenuItem } from './ContextMenu'
import {
  clampColumnWidth,
  dropIndexAt,
  reorderedNames,
  type OrderedColumn,
} from '../lib/columnLayout'
import { headerTypeLine } from '../lib/cellColor'
import { useExplorer } from '../state/explorer'
import { useGrid } from '../state/grid'

/**
 * How far the pointer must travel before a header press becomes a drag.
 *
 * Without a threshold every click-to-sort would also register as a
 * zero-distance reorder, and the two gestures start identically.
 */
const DRAG_THRESHOLD = 4

interface Props {
  ordered: OrderedColumn[]
  template: string
}

/**
 * The header row, and the three gestures that share it: click to sort, drag to
 * reorder, drag the grip to resize.
 *
 * `ordered` is display order — the indices used for dragging and for
 * `headerRects` are positions in it, never canonical column indices. Sorting
 * and resizing both address columns by *name* instead, since those outlive a
 * reorder.
 *
 * @param props - `{ ordered: OrderedColumn[]; template: string }`
 *   - `ordered` — columns in display order, each carrying its canonical index; drag
 *     indices and `headerRects` positions are indices into this list.
 *   - `template` — the CSS `grid-template-columns` value, shared with the rows so the
 *     two stay aligned.
 * @returns `JSX.Element` — the header row, plus the column context menu when one is open.
 */
export function GridHeader({ ordered, template }: Props) {
  const {
    sort,
    toggleSort,
    layout,
    setColumnWidth,
    clearColumnWidth,
    setColumnOrder,
    resetLayout,
  } = useGrid()
  const selected = useExplorer((s) => s.selected)

  const rowRef = useRef<HTMLDivElement>(null)
  const [dropAt, setDropAt] = useState<number | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)

  // Live drag state is a ref, not state: pointermove fires far faster than a
  // useful render, and only the drop indicator needs to re-render.
  const drag = useRef<{ from: number; startX: number; moved: boolean } | null>(null)
  const resize = useRef<{ column: string; startX: number; startWidth: number } | null>(null)

  /** Screen bounds of each header cell, in display order. */
  const headerRects = () =>
    Array.from(rowRef.current?.querySelectorAll('[data-header-cell]') ?? []).map((el) => {
      const r = (el as HTMLElement).getBoundingClientRect()
      return { left: r.left, right: r.right }
    })

  // ---------------------------------------------------------------- resize

  const onResizeDown = (e: React.PointerEvent, column: string, index: number) => {
    // Stop the press from also starting a reorder on the header behind it.
    e.preventDefault()
    e.stopPropagation()
    const cell = headerRects()[index]
    resize.current = {
      column,
      startX: e.clientX,
      startWidth: cell ? cell.right - cell.left : 0,
    }
    e.currentTarget.setPointerCapture(e.pointerId)
  }

  const onResizeMove = (e: React.PointerEvent) => {
    const r = resize.current
    if (!r || !selected) return
    // Written straight to the store: the template has to change for the
    // columns to move, and there is one grid row per page, not per pixel.
    setColumnWidth(
      selected.schema,
      selected.table,
      r.column,
      clampColumnWidth(r.startWidth + (e.clientX - r.startX)),
    )
  }

  const endResize = (e: React.PointerEvent) => {
    if (!resize.current) return
    resize.current = null
    e.currentTarget.releasePointerCapture(e.pointerId)
  }

  // --------------------------------------------------------------- reorder

  const onHeaderDown = (e: React.PointerEvent, displayIndex: number) => {
    if (e.button !== 0) return
    drag.current = { from: displayIndex, startX: e.clientX, moved: false }
    e.currentTarget.setPointerCapture(e.pointerId)
  }

  const onHeaderMove = (e: React.PointerEvent) => {
    const d = drag.current
    if (!d) return
    if (!d.moved && Math.abs(e.clientX - d.startX) < DRAG_THRESHOLD) return
    d.moved = true
    setDropAt(dropIndexAt(e.clientX, headerRects()))
  }

  const onHeaderUp = (e: React.PointerEvent, col: string) => {
    const d = drag.current
    drag.current = null
    setDropAt(null)
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId)
    }
    if (!d || !selected) return

    // Under the threshold the gesture was a click, and a click sorts.
    if (!d.moved) {
      void toggleSort(selected.schema, selected.table, col, e.shiftKey)
      return
    }
    const to = dropIndexAt(e.clientX, headerRects())
    setColumnOrder(selected.schema, selected.table, reorderedNames(ordered, d.from, to))
  }

  // ------------------------------------------------------------------ menu

  const openMenu = (e: React.MouseEvent, column: string) => {
    e.preventDefault()
    const hasWidth = layout.widths[column] !== undefined
    const customised = Object.keys(layout.widths).length > 0 || layout.order.length > 0
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        {
          label: 'Reset this column’s width',
          disabled: !hasWidth || !selected,
          onSelect: () => {
            if (selected) clearColumnWidth(selected.schema, selected.table, column)
          },
        },
        {
          label: 'Reset column layout',
          hint: 'widths and order',
          disabled: !customised || !selected,
          separatorBefore: true,
          onSelect: () => {
            if (selected) resetLayout(selected.schema, selected.table)
          },
        },
      ],
    })
  }

  return (
    <>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />
      )}
      <div className="grid-header" style={{ gridTemplateColumns: template }} ref={rowRef}>
        <div className="grid-header__num">#</div>
        {ordered.map(({ col }, displayIndex) => {
          const sortIndex = sort.findIndex((k) => k.column === col.name)
          const key = sortIndex >= 0 ? sort[sortIndex] : null
          return (
            <div
              key={col.name}
              data-header-cell
              className={`grid-header__cell${
                dropAt === displayIndex ? ' grid-header__cell--drop-before' : ''
              }${
                dropAt === displayIndex + 1 && displayIndex === ordered.length - 1
                  ? ' grid-header__cell--drop-after'
                  : ''
              }`}
              role="columnheader"
              aria-sort={key ? (key.dir === 'asc' ? 'ascending' : 'descending') : 'none'}
              // A div, not a button, because a button cannot contain the
              // resize grip as an independently-pressable child. That costs
              // the native keyboard behaviour, so it is restored explicitly.
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key !== 'Enter' && e.key !== ' ') return
                e.preventDefault()
                if (selected) void toggleSort(selected.schema, selected.table, col.name, e.shiftKey)
              }}
              title={`${col.name} — ${col.dataType}\nClick to sort · Shift-click to add to the sort · Drag to reorder`}
              onPointerDown={(e) => onHeaderDown(e, displayIndex)}
              onPointerMove={onHeaderMove}
              onPointerUp={(e) => onHeaderUp(e, col.name)}
              onPointerCancel={() => {
                drag.current = null
                setDropAt(null)
              }}
              onContextMenu={(e) => openMenu(e, col.name)}
            >
              <div className="grid-header__name">
                {col.name}
                {key && (
                  <span className="grid-sort">
                    {key.dir === 'asc' ? '↑' : '↓'}
                    {/* The ordinal only matters once there is more than one key. */}
                    {sort.length > 1 && <span className="grid-sort__ord">{sortIndex + 1}</span>}
                  </span>
                )}
              </div>
              <div className="grid-header__type">{headerTypeLine(col)}</div>

              <span
                className="grid-header__resize"
                title="Drag to resize · double-click to reset"
                onPointerDown={(e) => onResizeDown(e, col.name, displayIndex)}
                onPointerMove={onResizeMove}
                onPointerUp={endResize}
                onPointerCancel={endResize}
                onDoubleClick={(e) => {
                  e.stopPropagation()
                  if (selected) clearColumnWidth(selected.schema, selected.table, col.name)
                }}
              />
            </div>
          )
        })}
      </div>
    </>
  )
}
