import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { EditorView } from '@codemirror/view'
import { tags as t } from '@lezer/highlight'

/**
 * CodeMirror theme built from the design tokens.
 *
 * The values are hard-coded rather than read from CSS variables because
 * CodeMirror compiles styles into a stylesheet at construction time and cannot
 * resolve `var()` for everything it computes. They mirror `theme/tokens.css`
 * exactly — if a token changes there, change it here too.
 */
const C = {
  bg: '#0c0f15',
  text: '#cdd6e4',
  secondary: '#8b99b0',
  dim: '#6b7a92',
  faint: '#4a566b',
  xfaint: '#3d475a',
  border: '#1f2633',
  borderFaint: '#1a212e',
  accent: '#4e9cd8',
  accentLight: '#7cb9e8',
  amber: '#d9b26a',
  green: '#4ec98a',
  red: '#ff5f57',
  selection: 'rgba(78,156,216,0.35)',
  activeLine: '#12161f',
} as const

export const pgscopeEditorTheme = EditorView.theme(
  {
    '&': {
      color: C.text,
      backgroundColor: C.bg,
      fontSize: '12.5px',
      height: '100%',
    },
    '.cm-content': {
      fontFamily: "'IBM Plex Mono', ui-monospace, Menlo, monospace",
      padding: '10px 0',
      caretColor: C.accent,
    },
    '.cm-scroller': {
      fontFamily: "'IBM Plex Mono', ui-monospace, Menlo, monospace",
      lineHeight: '1.55',
      overflow: 'auto',
    },
    '&.cm-focused': { outline: 'none' },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: C.accent, borderLeftWidth: '2px' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
      backgroundColor: C.selection,
    },
    '.cm-activeLine': { backgroundColor: C.activeLine },
    '.cm-gutters': {
      backgroundColor: C.bg,
      color: C.xfaint,
      border: 'none',
      borderRight: `1px solid ${C.borderFaint}`,
      minWidth: '38px',
    },
    '.cm-lineNumbers .cm-gutterElement': { padding: '0 8px 0 6px' },
    '.cm-activeLineGutter': { backgroundColor: C.activeLine, color: C.dim },
    '.cm-matchingBracket, &.cm-focused .cm-matchingBracket': {
      backgroundColor: 'rgba(78,156,216,0.18)',
      color: C.accentLight,
      outline: 'none',
    },
    '.cm-nonmatchingBracket': { color: C.red },
    '.cm-searchMatch': { backgroundColor: 'rgba(217,178,106,0.20)' },
    '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'rgba(217,178,106,0.38)' },
    '.cm-panels': {
      backgroundColor: '#0f131b',
      color: C.secondary,
      border: 'none',
      borderTop: `1px solid ${C.border}`,
      fontSize: '11px',
    },
    '.cm-panel input, .cm-panel button': {
      fontFamily: "'IBM Plex Mono', monospace",
      backgroundColor: '#12161e',
      color: C.text,
      border: `1px solid ${C.border}`,
      borderRadius: '4px',
      padding: '2px 6px',
    },
    // Autocomplete popup, styled like the app's raised panels.
    '.cm-tooltip': {
      backgroundColor: '#161c26',
      border: `1px solid ${C.border}`,
      borderRadius: '6px',
      boxShadow: '0 8px 24px rgba(0,0,0,0.4)',
    },
    '.cm-tooltip.cm-tooltip-autocomplete > ul': {
      fontFamily: "'IBM Plex Mono', monospace",
      fontSize: '11.5px',
      maxHeight: '220px',
    },
    '.cm-tooltip.cm-tooltip-autocomplete > ul > li': { padding: '3px 8px' },
    '.cm-tooltip-autocomplete ul li[aria-selected]': {
      backgroundColor: 'rgba(78,156,216,0.18)',
      color: C.accentLight,
    },
    '.cm-completionLabel': { color: C.text },
    '.cm-completionDetail': { color: C.faint, fontStyle: 'normal', marginLeft: '8px' },
  },
  { dark: true },
)

/**
 * Syntax colours, reusing the palette the data grid uses for the same concepts:
 * keywords in accent blue, strings in amber, numbers in secondary.
 */
export const pgscopeHighlight = HighlightStyle.define([
  { tag: t.keyword, color: C.accent },
  { tag: t.operatorKeyword, color: C.accent },
  { tag: [t.string, t.special(t.string)], color: C.amber },
  { tag: [t.number, t.bool, t.null], color: C.secondary },
  { tag: [t.comment, t.lineComment, t.blockComment], color: C.xfaint, fontStyle: 'italic' },
  { tag: [t.variableName, t.propertyName], color: C.text },
  { tag: t.typeName, color: C.green },
  { tag: t.function(t.variableName), color: C.accentLight },
  { tag: t.operator, color: C.dim },
  { tag: t.punctuation, color: C.dim },
  { tag: t.invalid, color: C.red },
])

export const pgscopeSyntax = syntaxHighlighting(pgscopeHighlight)
