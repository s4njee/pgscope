import { describe, expect, it, vi } from 'vitest'

import { buildSavedMenu, duplicateName, moveLabel, moveTargets } from './savedMenu'
import type { SavedQuery } from './types'

/** A saved query whose path and body are irrelevant — the menu keys off the name. */
function q(name: string): SavedQuery {
  return { name, path: `/tmp/saved_queries/${name}.sql`, content: 'SELECT 1;' }
}

/** Fresh spies for every callback, so a test can assert which one an item invoked. */
function actions() {
  return {
    open: vi.fn(),
    rename: vi.fn(),
    duplicate: vi.fn(),
    remove: vi.fn(),
    move: vi.fn(),
    newFolder: vi.fn(),
    renameFolder: vi.fn(),
    copyName: vi.fn(),
  }
}

const labels = (items: { label: string }[]) => items.map((i) => i.label)

describe('buildSavedMenu', () => {
  it('offers the query actions on a query', () => {
    const items = buildSavedMenu({ kind: 'query', query: q('dau') }, [], actions())
    expect(labels(items)).toEqual(
      expect.arrayContaining(['Open', 'Rename…', 'Duplicate', 'Delete…']),
    )
  })

  it('puts delete last', () => {
    // It is the only irreversible entry; adjacent to Duplicate it would be one
    // slip away from the action people reach for most.
    const items = buildSavedMenu({ kind: 'query', query: q('dau') }, ['a'], actions())
    expect(labels(items).at(-1)).toBe('Delete…')
  })

  it('separates delete from the item above it', () => {
    const items = buildSavedMenu({ kind: 'query', query: q('dau') }, [], actions())
    expect(items.at(-1)?.separatorBefore).toBe(true)
  })

  it('marks destructive and naming actions with an ellipsis', () => {
    // The ellipsis is the promise that something will ask before it happens.
    const items = buildSavedMenu({ kind: 'query', query: q('dau') }, [], actions())
    for (const label of ['Rename…', 'Delete…', 'New folder…']) {
      expect(labels(items)).toContain(label)
    }
    expect(labels(items)).toContain('Duplicate') // acts immediately, so no ellipsis
  })

  it('runs the action it was given', () => {
    const a = actions()
    const query = q('dau')
    const items = buildSavedMenu({ kind: 'query', query }, [], a)
    items.find((i) => i.label === 'Delete…')?.run()
    expect(a.remove).toHaveBeenCalledWith(query)
  })

  it('offers a folder menu on a folder', () => {
    const items = buildSavedMenu({ kind: 'folder', path: 'reports' }, [], actions())
    expect(labels(items)).toContain('Rename folder…')
    // No Delete on a folder: folders disappear when they empty, so a delete
    // here would have to mean "delete everything inside", which is not what a
    // one-click menu item should do.
    expect(labels(items)).not.toContain('Delete…')
  })

  it('offers only New folder on empty background', () => {
    expect(labels(buildSavedMenu({ kind: 'background' }, [], actions()))).toEqual(['New folder…'])
  })
})

describe('move targets', () => {
  it('excludes the folder the query is already in', () => {
    expect(moveTargets(q('reports/dau'), ['reports', 'archive'])).toEqual(['', 'archive'])
  })

  it('offers the top level only when the query is not already there', () => {
    expect(moveTargets(q('dau'), ['reports'])).toEqual(['reports'])
    expect(moveTargets(q('reports/dau'), ['reports'])).toEqual([''])
  })

  it('labels the top level readably', () => {
    // '' would render as an empty menu row.
    expect(moveLabel('')).toBe('(top level)')
    expect(moveLabel('reports')).toBe('reports')
  })

  it('previews the resulting name as the hint', () => {
    const items = buildSavedMenu({ kind: 'query', query: q('dau') }, ['reports'], actions())
    const move = items.find((i) => i.label === 'Move to reports')
    expect(move?.hint).toBe('reports/dau')
  })

  it('moves by passing the destination folder through', () => {
    const a = actions()
    const query = q('dau')
    const items = buildSavedMenu({ kind: 'query', query }, ['reports'], a)
    items.find((i) => i.label === 'Move to reports')?.run()
    expect(a.move).toHaveBeenCalledWith(query, 'reports')
  })
})

describe('duplicateName', () => {
  it('appends _copy', () => {
    expect(duplicateName(q('dau'), [q('dau')])).toBe('dau_copy')
  })

  it('keeps the duplicate in the same folder', () => {
    expect(duplicateName(q('reports/dau'), [q('reports/dau')])).toBe('reports/dau_copy')
  })

  it('counts up past an existing copy', () => {
    // The backend refuses to overwrite, so proposing a taken name would just
    // surface an error the user has to fix by hand.
    const existing = [q('dau'), q('dau_copy'), q('dau_copy_2')]
    expect(duplicateName(q('dau'), existing)).toBe('dau_copy_3')
  })

  it('only considers collisions in the same folder', () => {
    const existing = [q('reports/dau'), q('dau_copy')]
    expect(duplicateName(q('reports/dau'), existing)).toBe('reports/dau_copy')
  })
})
