import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { orderColumns } from '../lib/columnLayout'
import { GridHeader } from './GridHeader'
import { useExplorer } from '../state/explorer'
import { useGrid } from '../state/grid'
import type { ColumnMeta } from '../lib/types'

const gridLayoutSave = vi.fn(() => Promise.resolve())

vi.mock('../lib/ipc', () => ({
  ipc: {
    fetchPage: vi.fn(() => Promise.resolve({ rows: [], columns: [], page: 0, pageSize: 100 })),
    gridLayoutLoad: vi.fn(() => Promise.resolve({})),
    gridLayoutSave: (...args: unknown[]) => gridLayoutSave(...(args as [])),
  },
  toAppError: (e: unknown) => ({ code: 'query', message: String(e), detail: null, sqlstate: null }),
}))

/**
 * A minimal column; only `name` distinguishes headers in these tests.
 *
 * @param name - `string` — the column name, which is also what the layout is keyed by.
 * @returns `ColumnMeta` — a `text` column with every flag false.
 */
function col(name: string): ColumnMeta {
  return { name, dataType: 'text', notNull: false, isPk: false, isFk: false }
}

const COLUMNS = [col('a'), col('b'), col('c')]

/**
 * jsdom does no layout, so every rect is zero. The drop logic is entirely
 * geometric, so the headers are given real bounds here: 100px wide, side by
 * side, matching the order they render in.
 *
 * Takes no arguments — it spies on the prototype, so it covers every element.
 *
 * @returns `void` — on return, `getBoundingClientRect` reports header cell *i* spanning
 *   x 100*i* to 100(*i*+1); anything not a header cell reports the leftmost slot.
 */
function stubGeometry() {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
    this: HTMLElement,
  ) {
    const cells = Array.from(document.querySelectorAll('[data-header-cell]'))
    const i = cells.indexOf(this)
    const left = i < 0 ? 0 : i * 100
    return { left, right: left + 100, top: 0, bottom: 40, width: 100, height: 40, x: left, y: 0 } as DOMRect
  })
}

/**
 * jsdom does not implement `PointerEvent`, and testing-library's
 * `fireEvent.pointerDown` silently degrades to a plain `Event` — which drops
 * `clientX` and `button`, so every handler here would see `undefined` and bail.
 * A `MouseEvent` carries both and React dispatches purely on the type name.
 *
 * @param el - `Element` — the event target; the event bubbles from here.
 * @param type - `'pointerdown' | 'pointermove' | 'pointerup' | 'pointercancel'` — the
 *   pointer event name to dispatch.
 * @param init - `{ clientX?: number; button?: number; shiftKey?: boolean }` — `clientX`
 *   in the stubbed geometry's pixel space, `button` 0 for the left button, `shiftKey`
 *   for a multi-column sort. Defaults to `{}`.
 * @returns `void` — dispatched inside `act`, so React has flushed on return.
 */
function pointer(
  el: Element,
  type: 'pointerdown' | 'pointermove' | 'pointerup' | 'pointercancel',
  init: { clientX?: number; button?: number; shiftKey?: boolean } = {},
) {
  act(() => {
    fireEvent(el, new MouseEvent(type, { bubbles: true, cancelable: true, ...init }))
  })
}

/**
 * jsdom implements neither pointer-capture method.
 *
 * Takes no arguments.
 *
 * @returns `void` — on return the three capture methods exist as spies, with
 *   `hasPointerCapture` always reporting true so the release path is exercised.
 */
function stubPointerCapture() {
  Element.prototype.setPointerCapture = vi.fn()
  Element.prototype.releasePointerCapture = vi.fn()
  Element.prototype.hasPointerCapture = vi.fn(() => true)
}

/**
 * Seeds both stores the header reads from — the explorer's selected table and
 * the grid's layout — before rendering, since the component pulls the saved
 * order and the drag handlers write back through them. `order` is that saved
 * order, empty meaning the natural one.
 *
 * @param order - `string[]` — the saved display order by column name; `[]` (the default)
 *   means the natural order of `COLUMNS`.
 * @returns `RenderResult` — testing-library's handle for the rendered header.
 */
function renderHeader(order: string[] = []) {
  act(() => {
    useExplorer.setState({ selected: { schema: 'public', table: 'events' } })
    useGrid.setState({ layout: { widths: {}, order }, sort: [] })
  })
  const ordered = orderColumns(COLUMNS, order)
  return render(<GridHeader ordered={ordered} template="44px 100px 100px 100px" />)
}

/**
 * The rendered header cells.
 *
 * Takes no arguments — it queries the document, so it must run after a render.
 *
 * @returns `HTMLElement[]` — the header cells in display order.
 */
const headers = () => Array.from(document.querySelectorAll('[data-header-cell]')) as HTMLElement[]

/**
 * The resize grip inside a header cell.
 *
 * @param i - `number` — the header's zero-based position in display order.
 * @returns `Element` — that header's grip; asserted non-null, since a missing grip is a
 *   test failure rather than a case to handle.
 */
const grip = (i: number) => headers()[i].querySelector('.grid-header__resize')!

/**
 * The column order the component has written back to the grid store.
 *
 * Takes no arguments.
 *
 * @returns `string[]` — column names in saved display order; `[]` when nothing has been
 *   reordered.
 */
const savedOrder = () => useGrid.getState().layout.order

beforeEach(() => {
  vi.restoreAllMocks()
  gridLayoutSave.mockClear()
  stubGeometry()
  stubPointerCapture()
  document.body.innerHTML = ''
})

describe('click still sorts', () => {
  it('sorts when the pointer does not move', () => {
    const toggleSort = vi.fn(() => Promise.resolve())
    renderHeader()
    act(() => {
      useGrid.setState({ toggleSort })
    })

    const h = headers()[0]
    pointer(h, 'pointerdown', { clientX: 50, button: 0 })
    pointer(h, 'pointerup', { clientX: 50 })

    expect(toggleSort).toHaveBeenCalledWith('public', 'events', 'a', false)
    expect(gridLayoutSave).not.toHaveBeenCalled()
  })

  it('passes shift through for a multi-column sort', () => {
    const toggleSort = vi.fn(() => Promise.resolve())
    renderHeader()
    act(() => {
      useGrid.setState({ toggleSort })
    })

    const h = headers()[1]
    pointer(h, 'pointerdown', { clientX: 150, button: 0 })
    pointer(h, 'pointerup', { clientX: 150, shiftKey: true })

    expect(toggleSort).toHaveBeenCalledWith('public', 'events', 'b', true)
  })

  it('sorts from the keyboard', () => {
    // The header is a div so the resize grip can be its own press target,
    // which costs the native button behaviour unless it is put back.
    const toggleSort = vi.fn(() => Promise.resolve())
    renderHeader()
    act(() => {
      useGrid.setState({ toggleSort })
    })

    fireEvent.keyDown(headers()[2], { key: 'Enter' })
    expect(toggleSort).toHaveBeenCalledWith('public', 'events', 'c', false)
  })

  it('tolerates a tiny jitter without turning the click into a drag', () => {
    // A press-and-release rarely lands on the exact same pixel.
    const toggleSort = vi.fn(() => Promise.resolve())
    renderHeader()
    act(() => {
      useGrid.setState({ toggleSort })
    })

    const h = headers()[0]
    pointer(h, 'pointerdown', { clientX: 50, button: 0 })
    pointer(h, 'pointermove', { clientX: 52 })
    pointer(h, 'pointerup', { clientX: 52 })

    expect(toggleSort).toHaveBeenCalled()
    expect(savedOrder()).toEqual([])
  })
})

describe('drag reorders', () => {
  it('moves a column to the right of its neighbour', () => {
    renderHeader()
    const h = headers()[0]
    pointer(h, 'pointerdown', { clientX: 50, button: 0 })
    pointer(h, 'pointermove', { clientX: 180 })
    pointer(h, 'pointerup', { clientX: 180 })
    // Dropped past b's midpoint, so a lands between b and c.
    expect(savedOrder()).toEqual(['b', 'a', 'c'])
  })

  it('moves a column to the far left', () => {
    renderHeader()
    const h = headers()[2]
    pointer(h, 'pointerdown', { clientX: 250, button: 0 })
    pointer(h, 'pointermove', { clientX: 10 })
    pointer(h, 'pointerup', { clientX: 10 })
    expect(savedOrder()).toEqual(['c', 'a', 'b'])
  })

  it('persists the new order', () => {
    renderHeader()
    const h = headers()[0]
    pointer(h, 'pointerdown', { clientX: 50, button: 0 })
    pointer(h, 'pointermove', { clientX: 280 })
    pointer(h, 'pointerup', { clientX: 280 })
    expect(gridLayoutSave).toHaveBeenCalledWith('public.events', {
      widths: {},
      order: ['b', 'c', 'a'],
    })
  })

  it('composes with an order that was already customised', () => {
    renderHeader(['c', 'b', 'a'])
    const h = headers()[0] // 'c'
    pointer(h, 'pointerdown', { clientX: 50, button: 0 })
    pointer(h, 'pointermove', { clientX: 280 })
    pointer(h, 'pointerup', { clientX: 280 })
    expect(savedOrder()).toEqual(['b', 'a', 'c'])
  })

  it('shows a drop indicator while dragging', () => {
    renderHeader()
    const h = headers()[0]
    pointer(h, 'pointerdown', { clientX: 50, button: 0 })
    pointer(h, 'pointermove', { clientX: 180 })
    expect(document.querySelector('.grid-header__cell--drop-before')).toBeTruthy()
  })

  it('clears the indicator and changes nothing when the drag is cancelled', () => {
    renderHeader()
    const h = headers()[0]
    pointer(h, 'pointerdown', { clientX: 50, button: 0 })
    pointer(h, 'pointermove', { clientX: 180 })
    pointer(h, 'pointercancel')
    expect(document.querySelector('.grid-header__cell--drop-before')).toBeNull()
    expect(savedOrder()).toEqual([])
  })

  it('ignores a right-button press', () => {
    // That press is opening the context menu, not starting a drag.
    renderHeader()
    const h = headers()[0]
    pointer(h, 'pointerdown', { clientX: 50, button: 2 })
    pointer(h, 'pointermove', { clientX: 250 })
    pointer(h, 'pointerup', { clientX: 250 })
    expect(savedOrder()).toEqual([])
  })
})

describe('resize', () => {
  it('sets a width from the drag distance', () => {
    renderHeader()
    pointer(grip(0), 'pointerdown', { clientX: 100, button: 0 })
    pointer(grip(0), 'pointermove', { clientX: 160 })
    // Started at the stubbed 100px width, dragged 60px right.
    expect(useGrid.getState().layout.widths.a).toBe(160)
  })

  it('clamps a drag that would collapse the column', () => {
    // A zero-width column takes its own grip with it, so the user could never
    // drag it back.
    renderHeader()
    pointer(grip(1), 'pointerdown', { clientX: 200, button: 0 })
    pointer(grip(1), 'pointermove', { clientX: 0 })
    expect(useGrid.getState().layout.widths.b).toBeGreaterThan(0)
  })

  it('does not also start a reorder', () => {
    // The grip sits inside the header, so without stopping propagation every
    // resize would reorder the column too.
    renderHeader()
    pointer(grip(0), 'pointerdown', { clientX: 100, button: 0 })
    pointer(grip(0), 'pointermove', { clientX: 260 })
    pointer(grip(0), 'pointerup', { clientX: 260 })
    expect(savedOrder()).toEqual([])
  })

  it('resets a width on double-click', () => {
    renderHeader()
    act(() => {
      useGrid.setState({ layout: { widths: { a: 300 }, order: [] } })
    })
    act(() => {
      fireEvent.doubleClick(grip(0))
    })
    expect(useGrid.getState().layout.widths.a).toBeUndefined()
  })
})

describe('header context menu', () => {
  it('offers a reset for a customised layout', () => {
    renderHeader()
    act(() => {
      useGrid.setState({ layout: { widths: { a: 300 }, order: [] } })
    })
    fireEvent.contextMenu(headers()[0], { clientX: 10, clientY: 10 })

    const reset = screen.getByText('Reset column layout').closest('button')!
    expect(reset.hasAttribute('disabled')).toBe(false)
    fireEvent.click(reset)
    expect(useGrid.getState().layout).toEqual({ widths: {}, order: [] })
  })

  it('disables the reset when nothing has been customised', () => {
    renderHeader()
    fireEvent.contextMenu(headers()[0], { clientX: 10, clientY: 10 })
    expect(
      screen.getByText('Reset column layout').closest('button')!.hasAttribute('disabled'),
    ).toBe(true)
  })
})
