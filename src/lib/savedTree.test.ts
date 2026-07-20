import { describe, expect, it } from 'vitest'

import {
  buildSavedTree,
  folderOf,
  folderPaths,
  leafOf,
  nameInFolder,
  segmentsOf,
  type FolderNode,
  type TreeNode,
} from './savedTree'
import type { SavedQuery } from './types'

/** A saved query whose path and body are irrelevant — the tree is built from names. */
function q(name: string): SavedQuery {
  return { name, path: `/tmp/saved_queries/${name}.sql`, content: 'SELECT 1;' }
}

/** Compact shape for asserting on structure without the SavedQuery payloads. */
function shape(nodes: TreeNode[]): unknown {
  return nodes.map((n) =>
    n.kind === 'folder' ? { folder: n.label, children: shape(n.children) } : n.label,
  )
}

describe('name helpers', () => {
  it('splits a name into segments', () => {
    expect(segmentsOf('reports/daily/dau')).toEqual(['reports', 'daily', 'dau'])
    expect(segmentsOf('top')).toEqual(['top'])
  })

  it('ignores empty segments from a stray slash', () => {
    expect(segmentsOf('reports//dau')).toEqual(['reports', 'dau'])
  })

  it('reports the folder and leaf halves', () => {
    expect(folderOf('reports/daily/dau')).toBe('reports/daily')
    expect(leafOf('reports/daily/dau')).toBe('dau')
    // A top-level query has no folder, which must be '' rather than undefined —
    // it is concatenated into a new name.
    expect(folderOf('top')).toBe('')
    expect(leafOf('top')).toBe('top')
  })
})

describe('buildSavedTree', () => {
  it('keeps top-level queries at the root', () => {
    expect(shape(buildSavedTree([q('a'), q('b')]))).toEqual(['a', 'b'])
  })

  it('groups queries under their folder', () => {
    const tree = buildSavedTree([q('reports/dau'), q('reports/wau'), q('scratch')])
    expect(shape(tree)).toEqual([
      { folder: 'reports', children: ['dau', 'wau'] },
      'scratch',
    ])
  })

  it('creates each folder once no matter how many queries it holds', () => {
    const tree = buildSavedTree([q('r/a'), q('r/b'), q('r/c')])
    expect(tree).toHaveLength(1)
    expect((tree[0] as FolderNode).children).toHaveLength(3)
  })

  it('nests folders arbitrarily deep', () => {
    const tree = buildSavedTree([q('a/b/c/leaf')])
    expect(shape(tree)).toEqual([
      { folder: 'a', children: [{ folder: 'b', children: [{ folder: 'c', children: ['leaf'] }] }] },
    ])
  })

  it('records the full path on each folder, not just its label', () => {
    // The path is the collapse key; using the bare label would collapse every
    // folder that happens to share a name with another one.
    const tree = buildSavedTree([q('a/shared/x'), q('b/shared/y')])
    const a = tree[0] as FolderNode
    const b = tree[1] as FolderNode
    expect((a.children[0] as FolderNode).path).toBe('a/shared')
    expect((b.children[0] as FolderNode).path).toBe('b/shared')
  })

  it('sorts folders before queries at every level', () => {
    // A flat sort by full path would put `scratch` between `reports/…` entries;
    // a folder hidden between two queries is hard to spot.
    const tree = buildSavedTree([q('scratch'), q('reports/dau'), q('archive')])
    expect(shape(tree)).toEqual([
      { folder: 'reports', children: ['dau'] },
      'archive',
      'scratch',
    ])
  })

  it('sorts nested levels too', () => {
    const tree = buildSavedTree([q('r/z'), q('r/a'), q('r/sub/x')])
    expect(shape(tree)).toEqual([
      { folder: 'r', children: [{ folder: 'sub', children: ['x'] }, 'a', 'z'] },
    ])
  })

  it('shows an empty folder that holds no queries', () => {
    // Otherwise "New folder" creates something the sidebar never displays.
    expect(shape(buildSavedTree([], ['reports']))).toEqual([{ folder: 'reports', children: [] }])
  })

  it('does not duplicate a folder that also has queries in it', () => {
    const tree = buildSavedTree([q('reports/dau')], ['reports'])
    expect(shape(tree)).toEqual([{ folder: 'reports', children: ['dau'] }])
  })

  it('creates the intermediate folders of a nested empty folder', () => {
    expect(shape(buildSavedTree([], ['a/b']))).toEqual([
      { folder: 'a', children: [{ folder: 'b', children: [] }] },
    ])
  })

  it('tolerates an empty list and an empty name', () => {
    expect(buildSavedTree([])).toEqual([])
    expect(buildSavedTree([q('')])).toEqual([])
  })

  it('keeps the original query on each leaf so actions have its path', () => {
    const tree = buildSavedTree([q('reports/dau')])
    const leaf = (tree[0] as FolderNode).children[0]
    expect(leaf.kind).toBe('query')
    if (leaf.kind === 'query') {
      expect(leaf.query.name).toBe('reports/dau')
      expect(leaf.query.path).toContain('reports/dau.sql')
    }
  })
})

describe('folderPaths', () => {
  it('lists every folder including nested ones', () => {
    const tree = buildSavedTree([q('a/b/x'), q('c/y'), q('top')])
    expect(folderPaths(tree)).toEqual(['a', 'a/b', 'c'])
  })

  it('is empty when nothing is foldered', () => {
    expect(folderPaths(buildSavedTree([q('a'), q('b')]))).toEqual([])
  })
})

describe('nameInFolder', () => {
  it('moves a query between folders by replacing the folder part', () => {
    expect(nameInFolder('reports/dau', 'archive')).toBe('archive/dau')
    expect(nameInFolder('dau', 'archive')).toBe('archive/dau')
  })

  it('moves a query to the top level with an empty folder', () => {
    // The leading-slash bug: `'' + '/' + leaf` would produce `/dau`, which the
    // backend then sanitises into something surprising.
    expect(nameInFolder('reports/dau', '')).toBe('dau')
  })
})
