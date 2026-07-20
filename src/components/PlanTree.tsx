import { useState } from 'react'

import { formatMs, groupDigits } from '../lib/format'
import type { ExplainResult, PlanNode } from '../lib/types'

/**
 * How badly a node's row estimate missed, in both directions. Planner
 * misestimates are the usual root cause behind a bad plan, so they get their
 * own visual treatment rather than being buried in the detail rows.
 *
 * @param ratio - `number | null` — actual rows over planned rows: above 1 the planner
 *   under-estimated, below 1 it over-estimated. `null` without ANALYZE.
 * @returns `{ level: 'none' | 'warn' | 'bad'; label: string }` — `'none'` with an empty
 *   label under a 10× miss, `'warn'` under 100×, `'bad'` at or above it.
 */
function misestimation(ratio: number | null): { level: 'none' | 'warn' | 'bad'; label: string } {
  if (ratio === null) return { level: 'none', label: '' }
  const factor = ratio >= 1 ? ratio : 1 / ratio
  if (!Number.isFinite(factor) || factor < 10) return { level: 'none', label: '' }

  const rounded = factor >= 100 ? Math.round(factor) : Math.round(factor * 10) / 10
  const direction = ratio > 1 ? 'under' : 'over'
  return {
    level: factor >= 100 ? 'bad' : 'warn',
    label: `${groupDigits(rounded)}× ${direction}-estimated`,
  }
}

/**
 * Scans that read everything are worth pointing at when they're expensive.
 *
 * @param node - `PlanNode` — the node to classify; only its `nodeType` is read.
 * @returns `boolean` — true for a `Seq Scan` or `Parallel Seq Scan`.
 */
function isSeqScan(node: PlanNode): boolean {
  return node.nodeType === 'Seq Scan' || node.nodeType === 'Parallel Seq Scan'
}

/**
 * The row count for a node's label. Estimates are prefixed `~`; without
 * ANALYZE that is all there is. A repeated node reports rows *per* iteration,
 * so the loop or worker count is spelled out rather than multiplied in —
 * the per-iteration figure is what compares against the estimate.
 *
 * @param node - `PlanNode` — read for `actualRows`, `planRows`, `actualLoops`, and
 *   `parallel`; a `null` `actualRows` means the plan was not executed.
 * @returns `string` — digit-grouped rows: `'—'` when neither figure exists, `~`-prefixed
 *   for an estimate, and suffixed `× N loops`/`× N workers` when repeated.
 */
function rows(node: PlanNode): string {
  if (node.actualRows === null) {
    return node.planRows === null ? '—' : `~${groupDigits(Math.round(node.planRows))}`
  }
  const actual = groupDigits(Math.round(node.actualRows))
  const loops = node.actualLoops ?? 1
  if (loops <= 1) return actual
  const unit = node.parallel ? 'workers' : 'loops'
  return `${actual} × ${groupDigits(Math.round(loops))} ${unit}`
}

/**
 * Under a Gather, `loops` counts parallel workers, so the summed time is CPU
 * across workers — it can exceed the query's wall-clock execution time. Saying
 * so beats printing a number that looks impossible.
 *
 * @param node - `PlanNode` — read for `selfTimeMs` (milliseconds, exclusive of children),
 *   `actualLoops`, and `parallel`. Only called once `selfTimeMs` is known non-null.
 * @returns `{ text: string; title: string }` — `text` is the short label, carrying a
 *   `cpu` suffix when the figure is summed across workers; `title` is the tooltip
 *   spelling out the worker count and the rough per-worker wall time.
 */
function timeLabel(node: PlanNode): { text: string; title: string } {
  const ms = formatMs(node.selfTimeMs as number, 2)
  const loops = node.actualLoops ?? 1
  if (node.parallel && loops > 1) {
    return {
      text: `${ms} cpu`,
      title: `${ms} of CPU time summed across ${Math.round(loops)} parallel workers — roughly ${formatMs(
        (node.selfTimeMs as number) / loops,
        2,
      )} of wall time each`,
    }
  }
  return { text: ms, title: `${ms} spent in this node, excluding its children` }
}

interface NodeProps {
  node: PlanNode
  result: ExplainResult
  depth: number
  collapsed: Record<string, boolean>
  toggle: (id: string) => void
}

/**
 * One plan node and, recursively, its children.
 *
 * `result` is threaded down for the tree-wide maxima the weight bar scales
 * against, and collapse state is held by the root rather than per row so
 * toggling doesn't reset when the tree re-renders.
 *
 * @param props - `{ node: PlanNode; result: ExplainResult; depth: number;
 *   collapsed: Record<string, boolean>; toggle: (id: string) => void }`
 *   - `node` — the node to render, along with its `children`.
 *   - `result` — the whole plan, for `analyzed` and the tree-wide `maxSelfTimeMs` /
 *     `maxSelfCost` the weight bar is scaled against.
 *   - `depth` — nesting level, 0 at the root; each level past it indents 18px.
 *   - `collapsed` — node id to collapsed flag, held by the root; a missing key is open.
 *   - `toggle` — flips the collapsed flag for the given node id.
 * @returns `JSX.Element` — the node's card and, unless collapsed, its subtree.
 */
function PlanNodeRow({ node, result, depth, collapsed, toggle }: NodeProps) {
  const isCollapsed = collapsed[node.id] ?? false
  const hasChildren = node.children.length > 0

  // Bars are scaled against the heaviest node so the eye lands on it. Time when
  // we have it, cost otherwise.
  const analyzed = result.analyzed && node.selfTimeMs !== null
  const value = analyzed ? node.selfTimeMs : node.selfCost
  const max = analyzed ? result.maxSelfTimeMs : result.maxSelfCost
  const share = value !== null && max !== null && max > 0 ? value / max : 0
  const isHeaviest = share >= 0.999 && share > 0

  const mis = misestimation(node.rowRatio)
  const flagSeqScan = isSeqScan(node) && share > 0.25

  return (
    <div className="plan-node" style={{ marginLeft: depth === 0 ? 0 : 18 }}>
      <div className={`plan-card${isHeaviest ? ' plan-card--heaviest' : ''}`}>
        <div className="plan-card__head">
          {hasChildren ? (
            <button
              className="plan-card__caret"
              onClick={() => toggle(node.id)}
              title={isCollapsed ? 'Expand' : 'Collapse'}
            >
              {isCollapsed ? '▸' : '▾'}
            </button>
          ) : (
            <span className="plan-card__caret plan-card__caret--leaf">·</span>
          )}

          <span className="plan-card__type">{node.nodeType}</span>

          {isHeaviest && <span className="plan-badge plan-badge--hot">slowest</span>}
          {mis.level !== 'none' && (
            <span className={`plan-badge plan-badge--${mis.level === 'bad' ? 'hot' : 'warn'}`}>
              {mis.label}
            </span>
          )}
          {flagSeqScan && <span className="plan-badge plan-badge--warn">seq scan</span>}

          <div className="spacer" />

          <span className="plan-card__rows">{rows(node)} rows</span>
          {analyzed ? (
            (() => {
              const t = timeLabel(node)
              return (
                <span className="plan-card__time" title={t.title}>
                  {t.text}
                </span>
              )
            })()
          ) : (
            node.selfCost !== null && (
              <span className="plan-card__time">cost {Math.round(node.selfCost)}</span>
            )
          )}
        </div>

        {/* Share of the total, so relative weight is readable at a glance. */}
        <div className="plan-bar">
          <div
            className={`plan-bar__fill${isHeaviest ? ' plan-bar__fill--hot' : ''}`}
            style={{ width: `${Math.max(share * 100, share > 0 ? 1.5 : 0)}%` }}
          />
        </div>

        {node.details.length > 0 && !isCollapsed && (
          <div className="plan-details">
            {node.details.map(([key, value]) => (
              <div className="plan-detail" key={key}>
                <span className="plan-detail__key">{key}</span>
                <span className="plan-detail__value">{value}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      {hasChildren && !isCollapsed && (
        <div className="plan-children">
          {node.children.map((child) => (
            <PlanNodeRow
              key={child.id}
              node={child}
              result={result}
              depth={depth + 1}
              collapsed={collapsed}
              toggle={toggle}
            />
          ))}
        </div>
      )}
    </div>
  )
}

/**
 * An EXPLAIN result: timing summary above, the node tree below.
 *
 * @param props - `{ result: ExplainResult }`
 *   - `result` — the parsed plan. `analyzed` false means estimates only; the planning
 *     and execution times are milliseconds and are omitted when `null`.
 * @returns `JSX.Element` — the summary strip and the scrollable node tree. Collapse
 *   state lives here so it survives a re-render of the tree.
 */
export function PlanTree({ result }: { result: ExplainResult }) {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({})
  const toggle = (id: string) =>
    setCollapsed((c) => ({ ...c, [id]: !(c[id] ?? false) }))

  return (
    <div className="plan-pane">
      <div className="plan-summary">
        <span className="plan-summary__mode">
          {result.analyzed ? 'EXPLAIN ANALYZE' : 'EXPLAIN'}
        </span>
        {result.planningTimeMs !== null && (
          <span>
            planning <span className="plan-summary__value">{formatMs(result.planningTimeMs, 2)}</span>
          </span>
        )}
        {result.executionTimeMs !== null && (
          <span>
            execution{' '}
            <span className="plan-summary__value">{formatMs(result.executionTimeMs, 2)}</span>
          </span>
        )}
        {!result.analyzed && <span className="plan-summary__note">estimates only — not executed</span>}
        {result.rolledBack && (
          <span className="plan-summary__note" title="The statement ran inside a transaction that was rolled back, so nothing was written">
            executed in a rolled-back transaction
          </span>
        )}
      </div>

      <div className="plan-scroll">
        <PlanNodeRow
          node={result.plan}
          result={result}
          depth={0}
          collapsed={collapsed}
          toggle={toggle}
        />
      </div>
    </div>
  )
}
