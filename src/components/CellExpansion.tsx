import { useEffect, useState } from 'react'

import { JSON_TOKEN_COLOR, tokenizeJson } from '../lib/jsonHighlight'
import { ipc, toAppError } from '../lib/ipc'
import { groupDigits, prettyBytes } from '../lib/format'
import type { AppError, CellValue, ColumnMeta, PageRequest } from '../lib/types'

/**
 * Syntax-coloured JSON. The text arrives already pretty-printed by Postgres.
 *
 * @param props - `{ text: string }`
 *   - `text` — the JSON document, already indented; whitespace is preserved verbatim.
 * @returns `JSX.Element` — a `<pre>` of coloured token spans.
 */
function JsonBody({ text }: { text: string }) {
  const tokens = tokenizeJson(text)
  return (
    <pre className="cellx__body selectable">
      {tokens.map((t, i) => (
        <span key={i} style={{ color: JSON_TOKEN_COLOR[t.kind] }}>
          {t.text}
        </span>
      ))}
    </pre>
  )
}

interface Props {
  column: ColumnMeta
  /** Primary-key values for the row, when the table has one. */
  pk: { column: string; value: string }[]
  page: PageRequest
  rowIndex: number
  columnCount: number
  onClose: () => void
}

/**
 * The panel that opens under a row when a cell is expanded.
 *
 * Re-fetches the value rather than using the grid's copy: the grid caps cells
 * at 8KB, and json arrives minified there.
 *
 * @param props - `{ column: ColumnMeta; pk: { column: string; value: string }[];
 *   page: PageRequest; rowIndex: number; columnCount: number; onClose: () => void }`
 *   - `column` — the expanded cell's column; its `dataType` is the fallback until the
 *     re-fetch returns one.
 *   - `pk` — primary-key column/value pairs for the row, empty when the table has none.
 *   - `page` — the schema/table/page/sort/filter the row was drawn from; the fetch has to
 *     repeat it to land on the same row.
 *   - `rowIndex` — the row's zero-based position *within the current page*, not the table.
 *   - `columnCount` — canonical column count; the panel spans that many grid columns plus
 *     the row-number gutter.
 *   - `onClose` — collapses the panel.
 * @returns `JSX.Element` — the full-width expansion row, showing a loading, error, `NULL`,
 *   JSON, or plain-text body depending on what came back.
 */
export function CellExpansion({
  column,
  pk,
  page,
  rowIndex,
  columnCount,
  onClose,
}: Props) {
  const [cell, setCell] = useState<CellValue | null>(null)
  const [error, setError] = useState<AppError | null>(null)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    let cancelled = false
    setCell(null)
    setError(null)

    ipc
      .fetchCell({ column: column.name, pk, page, rowIndex })
      .then((c) => !cancelled && setCell(c))
      .catch((e) => !cancelled && setError(toAppError(e)))

    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [column.name, rowIndex, page.schema, page.table, page.page])

  const copy = async () => {
    if (!cell?.value) return
    try {
      await navigator.clipboard.writeText(cell.value)
      setCopied(true)
      setTimeout(() => setCopied(false), 1200)
    } catch {
      /* clipboard unavailable */
    }
  }

  return (
    <div className="cellx" style={{ gridColumn: `1 / span ${columnCount + 1}` }}>
      <div className="cellx__head">
        <span className="cellx__col">{column.name}</span>
        <span className="cellx__type">{cell?.dataType ?? column.dataType}</span>

        {cell && cell.value !== null && (
          <span className="cellx__size">{prettyBytes(cell.totalBytes)}</span>
        )}
        {cell?.truncated && (
          <span className="cellx__warn">
            showing the first 4 MB of {groupDigits(cell.totalBytes)} bytes
          </span>
        )}
        {cell && !cell.locatedByPk && (
          <span
            className="cellx__warn"
            title="This table has no primary key, so the row was located by its position in the page. Concurrent changes could shift it."
          >
            located by position
          </span>
        )}

        <div className="spacer" />
        {cell?.value != null && (
          <button className="cellx__action" onClick={() => void copy()}>
            {copied ? 'copied' : 'copy'}
          </button>
        )}
        <button className="cellx__action" onClick={onClose} title="Collapse (Esc)">
          ▴ close
        </button>
      </div>

      {error && <div className="cellx__error">{error.message}</div>}
      {!error && !cell && <div className="cellx__loading">loading…</div>}
      {cell && cell.value === null && <div className="cellx__null">NULL</div>}
      {cell?.value != null &&
        (cell.format === 'json' ? (
          <JsonBody text={cell.value} />
        ) : (
          <pre className="cellx__body cellx__body--text selectable">{cell.value}</pre>
        ))}
    </div>
  )
}
