import { ipc } from './ipc'
import { oneLine } from './format'
import type { ColumnMeta, PredicateOp, RowFormat } from './types'

/**
 * Builds the grid's context-menu actions.
 *
 * Kept out of the component so the item list — which actions appear, when they
 * are disabled, what they copy — is testable without rendering anything.
 */
export interface GridMenuTarget {
  schema: string
  table: string
  columns: ColumnMeta[]
  values: (string | null)[]
  /** Index of the column that was right-clicked. */
  columnIndex: number
  rowIndex: number
}

export interface GridMenuActions {
  copy: (text: string) => Promise<void>
  /** Append a predicate to the grid filter and re-run. */
  applyFilter: (predicate: string) => void
  expandCell: (rowIndex: number, column: string) => void
  /** Whether this cell can be opened inline. */
  canExpand: (column: ColumnMeta, value: string | null) => boolean
}

export interface BuiltMenuItem {
  label: string
  hint?: string
  disabled?: boolean
  separatorBefore?: boolean
  run: () => void | Promise<void>
}

/** Short preview of a value for a menu hint. */
export function valueHint(value: string | null): string {
  if (value === null) return 'NULL'
  if (value === '') return "''"
  return oneLine(value, 24)
}

/**
 * The menu for one right-clicked cell, in the order it is drawn.
 *
 * Copy and filter entries are phrased against the cell that was clicked, so the
 * label alone says what will happen — a NULL cell offers `IS NULL` rather than
 * an equality test that could never match.
 */
export function buildGridMenu(
  target: GridMenuTarget,
  actions: GridMenuActions,
): BuiltMenuItem[] {
  const { schema, table, columns, values, columnIndex, rowIndex } = target
  const column = columns[columnIndex]
  const value = values[columnIndex] ?? null

  const formatAndCopy = (format: RowFormat) => async () => {
    const text = await ipc.formatRow({ schema, table, columns, values, format })
    await actions.copy(text)
  }

  const filterBy = (op: PredicateOp) => async () => {
    const predicate = await ipc.valuePredicate(column, value, op)
    actions.applyFilter(predicate)
  }

  const items: BuiltMenuItem[] = [
    {
      label: 'Copy cell',
      hint: valueHint(value),
      // Copying the four characters "NULL" is never what was meant.
      disabled: value === null,
      run: () => actions.copy(value ?? ''),
    },
    {
      label: 'Expand cell',
      disabled: !actions.canExpand(column, value),
      run: () => actions.expandCell(rowIndex, column.name),
    },
    {
      label: 'Copy row as JSON',
      separatorBefore: true,
      run: formatAndCopy('json'),
    },
    { label: 'Copy row as CSV', run: formatAndCopy('csv') },
    { label: 'Copy row as INSERT', run: formatAndCopy('insert') },
    {
      label: value === null ? `Filter: ${column.name} IS NULL` : `Filter: ${column.name} = value`,
      hint: value === null ? undefined : valueHint(value),
      separatorBefore: true,
      run: filterBy('eq'),
    },
    {
      label:
        value === null ? `Filter: ${column.name} IS NOT NULL` : `Filter: ${column.name} <> value`,
      run: filterBy('noteq'),
    },
  ]

  return items
}
