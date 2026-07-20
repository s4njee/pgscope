/**
 * Turning the flat saved-query list into the sidebar's folder tree.
 *
 * The backend reports each query's name as its path under the saved-queries
 * directory (`reports/daily/dau`), always with forward slashes so this code
 * never has to know the host separator. Kept pure and separate from the
 * component so the grouping — which is where the fiddly cases live — is
 * testable without rendering anything.
 */

import type { SavedQuery } from './types'

export interface QueryNode {
  kind: 'query'
  /** Last path segment — what the sidebar shows. */
  label: string
  query: SavedQuery
}

export interface FolderNode {
  kind: 'folder'
  label: string
  /** Full path from the root, e.g. `reports/daily`. Used as the collapse key. */
  path: string
  children: TreeNode[]
}

export type TreeNode = FolderNode | QueryNode

/** Split a saved-query name into its segments, ignoring empty ones. */
export function segmentsOf(name: string): string[] {
  return name.split('/').filter((s) => s.length > 0)
}

/** The folder part of a name, or '' for a top-level query. */
export function folderOf(name: string): string {
  const parts = segmentsOf(name)
  return parts.slice(0, -1).join('/')
}

/** The last segment of a name — what a rename dialog should start with. */
export function leafOf(name: string): string {
  const parts = segmentsOf(name)
  return parts[parts.length - 1] ?? name
}

/**
 * Build the tree.
 *
 * Folders sort before queries at each level, then alphabetically — the ordering
 * a file browser uses, so a folder never hides between two queries. Sorting
 * happens here rather than relying on the backend's flat sort, because that one
 * orders by full path and would interleave `reports/a` with a top-level `s`.
 */
export function buildSavedTree(queries: SavedQuery[], folders: string[] = []): TreeNode[] {
  const root: FolderNode = { kind: 'folder', label: '', path: '', children: [] }

  /** Walk down (creating as needed) to the folder named by `parts`. */
  const descend = (parts: string[]): FolderNode => {
    let cursor = root
    for (const segment of parts) {
      const path = cursor.path ? `${cursor.path}/${segment}` : segment
      let next = cursor.children.find(
        (c): c is FolderNode => c.kind === 'folder' && c.label === segment,
      )
      if (!next) {
        next = { kind: 'folder', label: segment, path, children: [] }
        cursor.children.push(next)
      }
      cursor = next
    }
    return cursor
  }

  // Empty folders are listed separately by the backend, because they cannot be
  // inferred from the queries — and a "New folder" that doesn't appear until
  // something is moved into it is not usable.
  for (const folder of folders) descend(segmentsOf(folder))

  for (const query of queries) {
    const parts = segmentsOf(query.name)
    if (parts.length === 0) continue
    descend(parts.slice(0, -1)).children.push({
      kind: 'query',
      label: parts[parts.length - 1],
      query,
    })
  }

  sortLevel(root)
  return root.children
}

/** Sort in place, recursively — `buildSavedTree` owns the tree until it returns. */
function sortLevel(folder: FolderNode) {
  folder.children.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === 'folder' ? -1 : 1
    return a.label.localeCompare(b.label)
  })
  for (const child of folder.children) {
    if (child.kind === 'folder') sortLevel(child)
  }
}

/** Every folder path in the tree, for "move to folder" menus. */
export function folderPaths(nodes: TreeNode[]): string[] {
  const out: string[] = []
  const walk = (list: TreeNode[]) => {
    for (const node of list) {
      if (node.kind === 'folder') {
        out.push(node.path)
        walk(node.children)
      }
    }
  }
  walk(nodes)
  return out.sort()
}

/**
 * The name a query would have after being moved into `folder`.
 *
 * Empty `folder` means the top level. Used by both the rename dialog's default
 * and the move-to-folder action, so the two cannot disagree.
 */
export function nameInFolder(queryName: string, folder: string): string {
  const leaf = leafOf(queryName)
  return folder ? `${folder}/${leaf}` : leaf
}
