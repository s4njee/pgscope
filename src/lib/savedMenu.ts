import type { BuiltMenuItem } from './gridMenu'
import { folderOf, leafOf, nameInFolder } from './savedTree'
import type { SavedQuery } from './types'

/**
 * Builds the saved-queries context menu.
 *
 * Same split as [`buildGridMenu`]: which items appear, and when they are
 * disabled, is decided here so it can be tested without rendering — and so the
 * destructive entries can be asserted on directly.
 */
export type SavedMenuTarget =
  | { kind: 'query'; query: SavedQuery }
  | { kind: 'folder'; path: string }
  | { kind: 'background' }

export interface SavedMenuActions {
  open: (query: SavedQuery) => void
  rename: (query: SavedQuery) => void
  duplicate: (query: SavedQuery) => void
  remove: (query: SavedQuery) => void
  move: (query: SavedQuery, folder: string) => void
  newFolder: () => void
  renameFolder: (path: string) => void
  copyName: (name: string) => void
}

/** Where a query could be moved to: every folder except the one it is in. */
export function moveTargets(query: SavedQuery, folders: string[]): string[] {
  const current = folderOf(query.name)
  // The top level is a destination too, but only when the query isn't already
  // there — it is represented by the empty string, which sorts oddly, so it is
  // prepended rather than mixed into the list.
  const rest = folders.filter((f) => f !== current)
  return current === '' ? rest : ['', ...rest]
}

/** Label for a move destination, since '' would otherwise render as nothing. */
export function moveLabel(folder: string): string {
  return folder === '' ? '(top level)' : folder
}

/**
 * The menu for a right-click in the saved-queries tree.
 *
 * The three targets are genuinely different menus rather than one list with
 * items disabled: a right-click on empty space has nothing to act on, so
 * offering greyed-out Rename and Delete would only be noise.
 */
export function buildSavedMenu(
  target: SavedMenuTarget,
  folders: string[],
  actions: SavedMenuActions,
): BuiltMenuItem[] {
  if (target.kind === 'background') {
    return [{ label: 'New folder…', run: () => actions.newFolder() }]
  }

  if (target.kind === 'folder') {
    return [
      { label: 'Rename folder…', run: () => actions.renameFolder(target.path) },
      { label: 'New folder…', run: () => actions.newFolder() },
      {
        label: 'Copy folder path',
        hint: target.path,
        separatorBefore: true,
        run: () => actions.copyName(target.path),
      },
    ]
  }

  const query = target.query
  const items: BuiltMenuItem[] = [
    { label: 'Open', run: () => actions.open(query) },
    { label: 'Rename…', separatorBefore: true, run: () => actions.rename(query) },
    { label: 'Duplicate', run: () => actions.duplicate(query) },
  ]

  for (const folder of moveTargets(query, folders)) {
    items.push({
      label: `Move to ${moveLabel(folder)}`,
      hint: nameInFolder(query.name, folder),
      run: () => actions.move(query, folder),
    })
  }

  items.push(
    { label: 'New folder…', separatorBefore: true, run: () => actions.newFolder() },
    { label: 'Copy name', hint: leafOf(query.name), run: () => actions.copyName(query.name) },
    // Last, and behind a confirmation, because it is the only entry here that
    // cannot be undone.
    { label: 'Delete…', separatorBefore: true, run: () => actions.remove(query) },
  )
  return items
}

/**
 * A name for a duplicate that does not collide with an existing query.
 *
 * The backend refuses to rename onto an existing file, so offering `x_copy`
 * when `x_copy` already exists would just produce an error the user has to
 * resolve by hand.
 */
export function duplicateName(query: SavedQuery, existing: SavedQuery[]): string {
  const taken = new Set(existing.map((q) => q.name))
  const folder = folderOf(query.name)
  const leaf = leafOf(query.name)

  for (let n = 1; ; n++) {
    const candidate = nameInFolder(n === 1 ? `${leaf}_copy` : `${leaf}_copy_${n}`, folder)
    if (!taken.has(candidate)) return candidate
  }
}
