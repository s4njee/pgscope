import { describe, expect, it } from 'vitest'

import { tokenizeJson, type JsonTokenKind } from './jsonHighlight'

/** Kinds in order, dropping whitespace-only plain runs, for readable asserts. */
function kinds(src: string): JsonTokenKind[] {
  return tokenizeJson(src)
    .filter((t) => t.text.trim() !== '')
    .map((t) => t.kind)
}

/** The first token whose text matches exactly — how a case names one token to classify. */
function tokenFor(src: string, text: string) {
  return tokenizeJson(src).find((t) => t.text === text)
}

describe('tokenizeJson', () => {
  it('round-trips the input exactly', () => {
    // Highlighting must never alter the value being inspected.
    const src = '{\n    "path": "/pricing",\n    "n": 42\n}'
    expect(tokenizeJson(src).map((t) => t.text).join('')).toBe(src)
  })

  it('distinguishes keys from string values', () => {
    const src = '{"path": "/pricing"}'
    expect(tokenFor(src, '"path"')?.kind).toBe('key')
    expect(tokenFor(src, '"/pricing"')?.kind).toBe('string')
  })

  it('classifies numbers, booleans and null', () => {
    const src = '{"a": 42, "b": -1.5e3, "c": true, "d": false, "e": null}'
    expect(tokenFor(src, '42')?.kind).toBe('number')
    expect(tokenFor(src, '-1.5e3')?.kind).toBe('number')
    expect(tokenFor(src, 'true')?.kind).toBe('boolean')
    expect(tokenFor(src, 'false')?.kind).toBe('boolean')
    expect(tokenFor(src, 'null')?.kind).toBe('null')
  })

  it('does not colour braces or digits inside strings', () => {
    // The whole point of tokenising: a URL or embedded JSON in a value must
    // stay one string token, not get chopped up by a regex.
    const src = '{"url": "https://x.test/a?b=1&c={2}"}'
    const value = tokenFor(src, '"https://x.test/a?b=1&c={2}"')
    expect(value?.kind).toBe('string')
    // Exactly one string token — nothing leaked out of it.
    expect(tokenizeJson(src).filter((t) => t.kind === 'string')).toHaveLength(1)
    expect(tokenizeJson(src).filter((t) => t.kind === 'number')).toHaveLength(0)
  })

  it('handles escaped quotes inside strings', () => {
    const src = '{"q": "she said \\"hi\\""}'
    const strings = tokenizeJson(src).filter((t) => t.kind === 'string')
    expect(strings).toHaveLength(1)
    expect(strings[0].text).toBe('"she said \\"hi\\""')
  })

  it('handles a trailing backslash escape without running away', () => {
    const src = '{"p": "C:\\\\path\\\\"}'
    expect(tokenizeJson(src).map((t) => t.text).join('')).toBe(src)
  })

  it('does not treat a word containing a literal as that literal', () => {
    const src = '{"k": "nullable"}'
    // "nullable" is inside a string, so it stays one string token.
    expect(tokenFor(src, '"nullable"')?.kind).toBe('string')
    expect(tokenizeJson(src).some((t) => t.kind === 'null')).toBe(false)
  })

  it('marks structural punctuation', () => {
    expect(kinds('{"a": [1]}')).toEqual([
      'punctuation', // {
      'key',
      'punctuation', // :
      'punctuation', // [
      'number',
      'punctuation', // ]
      'punctuation', // }
    ])
  })

  it('treats a key as a key across newlines and indentation', () => {
    // jsonb_pretty puts the colon on the same line, but be tolerant.
    const src = '{\n    "path"\n    : "/x"\n}'
    expect(tokenFor(src, '"path"')?.kind).toBe('key')
  })

  it('handles nested objects and arrays', () => {
    const src = '{"a": {"b": [{"c": 1}]}}'
    const ks = tokenizeJson(src).filter((t) => t.kind === 'key').map((t) => t.text)
    expect(ks).toEqual(['"a"', '"b"', '"c"'])
  })

  it('handles empty and non-object input', () => {
    expect(tokenizeJson('')).toEqual([])
    expect(kinds('42')).toEqual(['number'])
    expect(kinds('"bare"')).toEqual(['string'])
    // Not valid JSON, but must not throw or lose text.
    expect(tokenizeJson('{oops').map((t) => t.text).join('')).toBe('{oops')
  })

  it('handles an unterminated string without looping forever', () => {
    const src = '{"a": "unterminated'
    expect(tokenizeJson(src).map((t) => t.text).join('')).toBe(src)
  })

  it('preserves whitespace so indentation survives', () => {
    const src = '{\n    "a": 1\n}'
    const joined = tokenizeJson(src).map((t) => t.text).join('')
    expect(joined).toBe(src)
    expect(joined).toContain('\n    ')
  })
})
