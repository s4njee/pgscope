import { describe, expect, it } from 'vitest'

import {
  candidateColumns,
  compactCount,
  formatLatency,
  formatMs,
  groupDigits,
  historyKeyword,
  oneLine,
  prettyBytes,
  relativeAge,
  rowRange,
  suggestedQueryName,
} from './format'

describe('compactCount', () => {
  it('reproduces the sidebar counts in the design', () => {
    // Straight from the design's DATABASE tree.
    expect(compactCount(48_213_904)).toBe('48.2M')
    expect(compactCount(2_100_000)).toBe('2.1M')
    expect(compactCount(910_000)).toBe('910K')
    expect(compactCount(31_800_000)).toBe('31.8M')
    expect(compactCount(6_400_000)).toBe('6.4M')
    expect(compactCount(1_200_000)).toBe('1.2M')
    expect(compactCount(214)).toBe('214')
  })

  it('drops a trailing .0', () => {
    expect(compactCount(500_000)).toBe('500K')
    expect(compactCount(2_000_000)).toBe('2M')
  })

  it('shows a dash for a never-analyzed table', () => {
    // reltuples is -1 when the table has never been ANALYZEd.
    expect(compactCount(-1)).toBe('—')
  })

  it('handles the boundaries', () => {
    expect(compactCount(0)).toBe('0')
    expect(compactCount(999)).toBe('999')
    expect(compactCount(1000)).toBe('1K')
  })
})

describe('groupDigits', () => {
  it('formats the design footer total', () => {
    expect(groupDigits(48_213_904)).toBe('48,213,904')
  })
})

describe('prettyBytes', () => {
  it('formats the design stats values', () => {
    expect(prettyBytes(12 * 1024 ** 3)).toBe('12 GB')
    expect(prettyBytes(Math.round(3.1 * 1024 ** 3))).toBe('3.1 GB')
  })

  it('handles small and zero sizes', () => {
    expect(prettyBytes(0)).toBe('0 bytes')
    expect(prettyBytes(512)).toBe('512 bytes')
    expect(prettyBytes(2048)).toBe('2 kB')
  })
})

describe('relativeAge', () => {
  it('formats the design stats age', () => {
    expect(relativeAge(41 * 60)).toBe('41 min ago')
  })

  it('formats short history ages like the design sidebar', () => {
    // The design shows `· 2m`, `· 9m`, `· 12m`, `· 31m`.
    expect(relativeAge(2 * 60, 'short')).toBe('2m')
    expect(relativeAge(9 * 60, 'short')).toBe('9m')
    expect(relativeAge(31 * 60, 'short')).toBe('31m')
  })

  it('handles a missing timestamp', () => {
    // last_autovacuum is NULL on a table autovacuum has never touched.
    expect(relativeAge(null)).toBe('never')
    expect(relativeAge(undefined)).toBe('never')
  })

  it('scales up through hours and days', () => {
    expect(relativeAge(3 * 3600)).toBe('3 hour ago')
    expect(relativeAge(2 * 86400, 'short')).toBe('2d')
  })
})

describe('formatMs / formatLatency', () => {
  it('matches the design footer and pill', () => {
    expect(formatMs(11.8)).toBe('11.8 ms')
    expect(formatMs(428.116, 3)).toBe('428.116 ms')
    expect(formatLatency(12)).toBe('12ms')
    expect(formatLatency(11.6)).toBe('12ms')
  })
})

describe('rowRange', () => {
  it('formats the design range', () => {
    // "rows 1–50 of 48,213,904" — an en dash, not a hyphen.
    expect(rowRange(0, 50, 50)).toBe('1–50')
    expect(rowRange(1, 50, 50)).toBe('51–100')
  })

  it('handles a partial last page', () => {
    expect(rowRange(2, 50, 4)).toBe('101–104')
  })

  it('handles an empty result', () => {
    expect(rowRange(0, 50, 0)).toBe('no rows')
  })
})

describe('historyKeyword', () => {
  it('splits the leading keyword the design highlights', () => {
    // The four entries in the design's HISTORY panel.
    expect(historyKeyword('\\d events')).toEqual({ keyword: '\\d', rest: 'events' })
    expect(historyKeyword('SELECT event_name, count(*)')).toEqual({
      keyword: 'SELECT',
      rest: 'event_name, count(*)',
    })
    expect(historyKeyword('\\timing on')).toEqual({ keyword: '\\timing', rest: 'on' })
    expect(historyKeyword('EXPLAIN ANALYZE SELECT 1')).toEqual({
      keyword: 'EXPLAIN ANALYZE',
      rest: 'SELECT 1',
    })
  })

  it('handles empty input', () => {
    expect(historyKeyword('   ')).toEqual({ keyword: '', rest: '' })
  })
})

describe('oneLine', () => {
  it('collapses whitespace and truncates', () => {
    expect(oneLine('SELECT\n  1,\n  2')).toBe('SELECT 1, 2')
    expect(oneLine('a'.repeat(80), 10)).toBe('a'.repeat(9) + '…')
  })
})

describe('suggestedQueryName', () => {
  it('names queries in the style of the design sidebar', () => {
    expect(suggestedQueryName('SELECT event_name, count(*) FROM events')).toBe(
      'event_name_count_events',
    )
  })

  it('drops SQL keywords and punctuation', () => {
    expect(suggestedQueryName("SELECT * FROM users WHERE plan = 'pro'")).toBe('users_plan_pro')
  })

  it('falls back for input with nothing usable', () => {
    expect(suggestedQueryName('SELECT * FROM')).toBe('query')
    expect(suggestedQueryName('')).toBe('query')
  })
})

describe('candidateColumns', () => {
  it('lays candidates out in columns, filling downward', () => {
    // 6 items in a narrow width: 2 columns of 3, read top-to-bottom.
    const out = candidateColumns(['aa', 'bb', 'cc', 'dd', 'ee', 'ff'], 10)
    const lines = out.trimEnd().split('\n')
    expect(lines).toHaveLength(3)
    expect(lines[0]).toBe('aa  dd')
    expect(lines[1]).toBe('bb  ee')
    expect(lines[2]).toBe('cc  ff')
  })

  it('pads to the widest candidate so columns align', () => {
    const out = candidateColumns(['a', 'bbbbbb', 'c', 'd'], 20)
    const lines = out.trimEnd().split('\n')
    // Column width is 6 + 2 padding, so the second column starts at offset 8.
    expect(lines[0].indexOf('c')).toBe(8)
  })

  it('uses a single column when candidates are wide', () => {
    const out = candidateColumns(['a_very_long_table_name', 'another_long_one'], 20)
    expect(out.trimEnd().split('\n')).toHaveLength(2)
  })

  it('handles one and zero candidates', () => {
    expect(candidateColumns(['events'])).toBe('events\n')
    expect(candidateColumns([])).toBe('')
  })

  it('always ends with a newline so the next prompt starts fresh', () => {
    expect(candidateColumns(['a', 'b']).endsWith('\n')).toBe(true)
  })
})
