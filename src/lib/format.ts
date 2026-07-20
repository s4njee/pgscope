/**
 * Display formatters. The design shows specific renderings ("48.2M", "12 GB",
 * "41 min ago", "48,213,904") — these produce exactly those.
 */

/** Sidebar row counts: `48.2M`, `910K`, `214`. */
export function compactCount(n: number): string {
  if (n < 0) return '—' // reltuples = -1: never analyzed
  if (n < 1000) return String(n)
  const units: [number, string][] = [
    [1e12, 'T'],
    [1e9, 'B'],
    [1e6, 'M'],
    [1e3, 'K'],
  ]
  for (const [scale, suffix] of units) {
    if (n >= scale) {
      const v = n / scale
      // Two significant-ish digits: 48.2M, but 214K not 214.0K.
      const s = v >= 100 ? Math.round(v).toString() : v.toFixed(1).replace(/\.0$/, '')
      return `${s}${suffix}`
    }
  }
  return String(n)
}

/** Footer totals and stats: `48,213,904`. */
export function groupDigits(n: number): string {
  return n.toLocaleString('en-US')
}

/** Stats panel sizes: `12 GB`, `3.1 GB`, `914 kB`. Mirrors pg_size_pretty. */
export function prettyBytes(bytes: number): string {
  const units = ['bytes', 'kB', 'MB', 'GB', 'TB']
  let value = bytes
  let i = 0
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024
    i++
  }
  if (i === 0) return `${Math.round(value)} ${units[0]}`
  const s = value >= 100 ? Math.round(value).toString() : value.toFixed(1).replace(/\.0$/, '')
  return `${s} ${units[i]}`
}

/** Stats + history ages: `41 min ago`, `2m`, `never`. */
export function relativeAge(seconds: number | null | undefined, style: 'long' | 'short' = 'long'): string {
  if (seconds === null || seconds === undefined) return style === 'long' ? 'never' : ''
  const s = Math.max(0, Math.floor(seconds))
  const table: [number, string, string][] = [
    [60, 'sec', 's'],
    [3600, 'min', 'm'],
    [86400, 'hour', 'h'],
    [86400 * 30, 'day', 'd'],
  ]
  if (s < 60) return style === 'long' ? `${s} sec ago` : `${s}s`
  for (let i = 1; i < table.length; i++) {
    const [limit, longUnit, shortUnit] = table[i]
    if (s < limit) {
      const divisor = table[i - 1][0]
      const v = Math.floor(s / divisor)
      return style === 'long' ? `${v} ${longUnit} ago` : `${v}${shortUnit}`
    }
  }
  const days = Math.floor(s / 86400)
  return style === 'long' ? `${days} day ago` : `${days}d`
}

/** Query timings: `11.8 ms`, `428.116 ms`. */
export function formatMs(ms: number, precision = 1): string {
  return `${ms.toFixed(precision)} ms`
}

/** Latency pill: `12ms`. */
export function formatLatency(ms: number): string {
  return `${Math.round(ms)}ms`
}

/** Footer range: `rows 1–50 of 48,213,904`. Uses an en dash, per the design. */
export function rowRange(page: number, pageSize: number, rowCount: number): string {
  if (rowCount === 0) return 'no rows'
  const start = page * pageSize + 1
  const end = page * pageSize + rowCount
  return `${groupDigits(start)}–${groupDigits(end)}`
}

/**
 * The leading keyword the history panel renders in accent colour:
 * `SELECT`, `\d`, `\timing`, `EXPLAIN ANALYZE`.
 */
export function historyKeyword(input: string): { keyword: string; rest: string } {
  const trimmed = input.trim()
  if (!trimmed) return { keyword: '', rest: '' }

  if (trimmed.startsWith('\\')) {
    const [kw, ...rest] = trimmed.split(/\s+/)
    return { keyword: kw, rest: rest.join(' ') }
  }

  // Two-word keywords the design shows highlighted as a unit.
  const twoWord = ['EXPLAIN ANALYZE', 'CREATE TABLE', 'CREATE INDEX', 'DROP TABLE', 'ALTER TABLE', 'INSERT INTO']
  const upper = trimmed.toUpperCase()
  for (const kw of twoWord) {
    if (upper.startsWith(kw + ' ') || upper === kw) {
      return { keyword: trimmed.slice(0, kw.length), rest: trimmed.slice(kw.length).trim() }
    }
  }

  const [kw, ...rest] = trimmed.split(/\s+/)
  return { keyword: kw, rest: rest.join(' ') }
}

/** Single-line preview for the history panel; collapses whitespace. */
export function oneLine(input: string, max = 60): string {
  const collapsed = input.replace(/\s+/g, ' ').trim()
  return collapsed.length > max ? collapsed.slice(0, max - 1) + '…' : collapsed
}

/**
 * A filename suggestion for saving a statement, in the style of the design's
 * saved queries (`dau_last_30d`, `top_events_hourly`): lowercase words from the
 * head of the statement, joined with underscores.
 */
export function suggestedQueryName(statement: string): string {
  const words = statement
    .replace(/[^\w\s]/g, ' ')
    .split(/\s+/)
    .filter(Boolean)
    .filter((w) => !/^(select|from|where|group|order|by|limit|the|and|a)$/i.test(w))
    .slice(0, 4)
    .map((w) => w.toLowerCase())
  return words.length > 0 ? words.join('_') : 'query'
}

/**
 * Lay candidates out in columns the way psql and shells do, filling
 * top-to-bottom then left-to-right within `width` characters.
 */
export function candidateColumns(values: string[], width = 80): string {
  if (values.length === 0) return ''

  const colWidth = Math.max(...values.map((v) => v.length)) + 2
  const columns = Math.max(1, Math.floor(width / colWidth))
  const rows = Math.ceil(values.length / columns)

  const lines: string[] = []
  for (let r = 0; r < rows; r++) {
    const cells: string[] = []
    for (let c = 0; c < columns; c++) {
      // Down each column first, so alphabetical order reads vertically.
      const i = c * rows + r
      if (i < values.length) cells.push(values[i].padEnd(colWidth))
    }
    lines.push(cells.join('').trimEnd())
  }
  return lines.join('\n') + '\n'
}
