import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ipc } from '../lib/ipc'
import { useExplorer } from './explorer'
import type { SchemaNode, TableMeta } from '../lib/types'

vi.mock('../lib/ipc', () => ({
  ipc: {
    schemaTree: vi.fn(),
    tableMeta: vi.fn(),
  },
  toAppError: (e: unknown) => ({
    code: 'query',
    message: String(e),
    detail: null,
    sqlstate: null,
  }),
}))

const schemaTree = ipc.schemaTree as unknown as ReturnType<typeof vi.fn>
const tableMeta = ipc.tableMeta as unknown as ReturnType<typeof vi.fn>

/** One schema with the given tables, plus a single view, as the tree returns. */
function tree(tables: string[], views: string[] = ['active_users']): SchemaNode[] {
  return [
    {
      name: 'public',
      tables: tables.map((name) => ({ name, kind: 'table', estRows: 100 })),
      views: views.map((name) => ({ name, kind: 'view', estRows: -1 })),
    } as SchemaNode,
  ]
}

/** Minimal metadata; only its identity matters to these tests. */
function meta(table: string): TableMeta {
  return {
    schema: 'public',
    name: table,
    kind: 'table',
    columns: [],
    indexes: [],
    stats: { estRows: null, totalBytes: null, indexBytes: null, lastAutovacuumSecs: null },
  }
}

beforeEach(() => {
  schemaTree.mockReset()
  tableMeta.mockReset()
  tableMeta.mockImplementation((_s: string, t: string) => Promise.resolve(meta(t)))
  useExplorer.setState({ tree: [], selected: null, meta: null, error: null })
})

describe('loading the tree', () => {
  it('auto-selects the first table when nothing is selected', async () => {
    schemaTree.mockResolvedValue(tree(['events', 'users']))
    await useExplorer.getState().loadTree()

    expect(useExplorer.getState().selected).toEqual({ schema: 'public', table: 'events' })
    expect(tableMeta).toHaveBeenCalledWith('public', 'events')
  })

  it('fetches metadata for a selection restored from disk', async () => {
    // The bug this pins: `selected` is persisted but `meta` is not, and the
    // metadata fetch used to live inside the auto-select branch — so a restored
    // selection reopened on the right table with an empty grid and "no
    // metadata", with nothing on screen explaining why.
    useExplorer.setState({ selected: { schema: 'public', table: 'users' }, meta: null })
    schemaTree.mockResolvedValue(tree(['events', 'users']))

    await useExplorer.getState().loadTree()

    expect(tableMeta).toHaveBeenCalledWith('public', 'users')
    expect(useExplorer.getState().meta?.name).toBe('users')
    // And it must not have wandered off to the default table.
    expect(useExplorer.getState().selected).toEqual({ schema: 'public', table: 'users' })
  })

  it('restores a selected view, not just a table', async () => {
    useExplorer.setState({ selected: { schema: 'public', table: 'active_users' }, meta: null })
    schemaTree.mockResolvedValue(tree(['events']))

    await useExplorer.getState().loadTree()
    expect(tableMeta).toHaveBeenCalledWith('public', 'active_users')
  })

  it('falls back to the default when the persisted table is gone', async () => {
    // A table dropped between sessions must not leave the app erroring on a
    // selection that cannot resolve.
    useExplorer.setState({ selected: { schema: 'public', table: 'dropped' }, meta: null })
    schemaTree.mockResolvedValue(tree(['events', 'users']))

    await useExplorer.getState().loadTree()
    expect(useExplorer.getState().selected).toEqual({ schema: 'public', table: 'events' })
    expect(tableMeta).not.toHaveBeenCalledWith('public', 'dropped')
  })

  it('falls back when the persisted schema is gone', async () => {
    useExplorer.setState({ selected: { schema: 'staging', table: 'events' }, meta: null })
    schemaTree.mockResolvedValue(tree(['events']))

    await useExplorer.getState().loadTree()
    expect(useExplorer.getState().selected).toEqual({ schema: 'public', table: 'events' })
  })

  it('surfaces a tree failure instead of selecting anything', async () => {
    schemaTree.mockRejectedValue(new Error('connection lost'))
    await useExplorer.getState().loadTree()

    expect(useExplorer.getState().error?.message).toContain('connection lost')
    expect(tableMeta).not.toHaveBeenCalled()
  })
})
