/**
 * Tokenise pretty-printed JSON for display.
 *
 * A tokeniser rather than a regex-replace: the naive approach colours the
 * *contents* of strings that happen to contain braces or digits, which is
 * exactly what jsonb values are full of (URLs, embedded JSON, timestamps).
 * Postgres has already formatted the text via `jsonb_pretty`, so this only
 * classifies — it never reflows.
 */
export type JsonTokenKind =
  | 'key'
  | 'string'
  | 'number'
  | 'boolean'
  | 'null'
  | 'punctuation'
  | 'plain'

export interface JsonToken {
  text: string
  kind: JsonTokenKind
}

const PUNCTUATION = new Set(['{', '}', '[', ']', ',', ':'])

/** Read a JSON string literal starting at `i` (which must be the opening `"`). */
function readString(src: string, i: number): { text: string; next: number } {
  let j = i + 1
  while (j < src.length) {
    const c = src[j]
    if (c === '\\') {
      j += 2 // an escape consumes the next char, including \" and \\
      continue
    }
    if (c === '"') {
      j += 1
      break
    }
    j += 1
  }
  return { text: src.slice(i, j), next: j }
}

/** True when the next non-whitespace character is a `:`, making this a key. */
function isKeyPosition(src: string, from: number): boolean {
  let j = from
  while (j < src.length && /\s/.test(src[j])) j += 1
  return src[j] === ':'
}

/**
 * Split JSON text into coloured runs, covering every character exactly once —
 * concatenating the tokens reproduces the input, so rendering them cannot lose
 * or reorder anything. Text that isn't valid JSON still comes back whole, as
 * `plain` runs, because a truncated or hand-edited cell must remain readable.
 */
export function tokenizeJson(src: string): JsonToken[] {
  const tokens: JsonToken[] = []
  let i = 0
  let pending = ''

  const flush = () => {
    if (pending) {
      tokens.push({ text: pending, kind: 'plain' })
      pending = ''
    }
  }
  const push = (text: string, kind: JsonTokenKind) => {
    flush()
    tokens.push({ text, kind })
  }

  while (i < src.length) {
    const c = src[i]

    if (c === '"') {
      const { text, next } = readString(src, i)
      push(text, isKeyPosition(src, next) ? 'key' : 'string')
      i = next
      continue
    }

    if (PUNCTUATION.has(c)) {
      push(c, 'punctuation')
      i += 1
      continue
    }

    // Literals and numbers, only when they start a token — so `true` inside a
    // bare word isn't picked up.
    const rest = src.slice(i)
    const literal = /^(true|false|null)\b/.exec(rest)
    if (literal) {
      push(literal[0], literal[0] === 'null' ? 'null' : 'boolean')
      i += literal[0].length
      continue
    }

    const number = /^-?\d+(\.\d+)?([eE][-+]?\d+)?/.exec(rest)
    if (number) {
      push(number[0], 'number')
      i += number[0].length
      continue
    }

    pending += c
    i += 1
  }

  flush()
  return tokens
}

/** CSS custom property for each token kind. */
export const JSON_TOKEN_COLOR: Record<JsonTokenKind, string> = {
  key: 'var(--accent-light)',
  string: 'var(--amber)',
  number: 'var(--green)',
  boolean: 'var(--accent-lighter)',
  null: 'var(--text-faint)',
  punctuation: 'var(--text-dim)',
  plain: 'var(--text-secondary)',
}
