import { columnBadge, shortType } from '../lib/cellColor'
import { groupDigits, prettyBytes, relativeAge } from '../lib/format'
import type { ColumnMeta } from '../lib/types'
import { useExplorer } from '../state/explorer'

/**
 * A column's single PK/FK/NN badge, or nothing when it has no role worth marking.
 *
 * @param props - `{ col: ColumnMeta }`
 *   - `col` — the column; only its `isPk`/`isFk`/`notNull` flags are read, and PK wins
 *     over FK, which wins over NN.
 * @returns `JSX.Element | null` — the badge span, or `null` for a plain nullable column.
 */
function Badge({ col }: { col: ColumnMeta }) {
  const badge = columnBadge(col)
  if (!badge) return null
  const cls = badge === 'PK' ? 'badge--pk' : badge === 'FK' ? 'badge--fk' : 'badge--nn'
  return <span className={`badge ${cls}`}>{badge}</span>
}

/**
 * The right rail: columns, indexes, and stats for the selected table.
 *
 * Takes no arguments — the selection and its metadata come from the explorer store.
 *
 * @returns `JSX.Element | null` — `null` when nothing is selected, a loading/empty
 *   placeholder while the metadata is in flight, otherwise the full rail. Stats render
 *   as `n/a` where Postgres reported no value.
 */
export function DetailsPanel() {
  const { meta, metaLoading, selected } = useExplorer()

  if (!selected) return null

  if (!meta) {
    return (
      <div className="details">
        <div className="details__empty">{metaLoading ? 'loading…' : 'no metadata'}</div>
      </div>
    )
  }

  const { stats } = meta

  return (
    <div className="details">
      <div className="details__header">
        <span className="details__name" title={meta.name}>
          {meta.name}
        </span>
        <span className="badge-kind">{meta.kind === 'view' ? 'VIEW' : 'TABLE'}</span>
      </div>

      <div className="details__section">COLUMNS</div>
      <div className="details__cols">
        {meta.columns.map((col) => (
          <div className="col-row" key={col.name}>
            <span className="col-row__name" title={col.name}>
              {col.name}
            </span>
            <span className="col-row__type" title={col.dataType}>
              {shortType(col.dataType)}
            </span>
            <Badge col={col} />
          </div>
        ))}
      </div>

      <div className="details__section">INDEXES</div>
      {meta.indexes.length === 0 ? (
        <div className="details__empty">none</div>
      ) : (
        <div className="details__indexes">
          {meta.indexes.map((idx) => (
            <div key={idx.name}>
              <div className="index-name">{idx.name}</div>
              <div className="index-def">{idx.definition}</div>
            </div>
          ))}
        </div>
      )}

      <div className="details__section">STATS</div>
      <div className="details__stats">
        <div className="stat-row">
          <span className="stat-row__label">est. rows</span>
          <span className="stat-row__value">
            {stats.estRows === null || stats.estRows < 0 ? 'n/a' : groupDigits(stats.estRows)}
          </span>
        </div>
        <div className="stat-row">
          <span className="stat-row__label">total size</span>
          <span className="stat-row__value">
            {stats.totalBytes === null ? 'n/a' : prettyBytes(stats.totalBytes)}
          </span>
        </div>
        <div className="stat-row">
          <span className="stat-row__label">indexes</span>
          <span className="stat-row__value">
            {stats.indexBytes === null ? 'n/a' : prettyBytes(stats.indexBytes)}
          </span>
        </div>
        <div className="stat-row">
          <span className="stat-row__label">last autovacuum</span>
          <span className="stat-row__value">{relativeAge(stats.lastAutovacuumSecs)}</span>
        </div>
      </div>
    </div>
  )
}
