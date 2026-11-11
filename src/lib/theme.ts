// Palette per chi non legge le variabili CSS (Monaco, xterm). Ogni chiave replica un token di
// src/styles/glass.css con lo stesso valore: `palette.surface1` = `--surface-1`, ecc.
// glass.test.ts controlla che i due file non divergano.
export const palette = {
  bg: '#0e1016', // --bg
  surface1: '#151821', // --surface-1 (sfondo dei pannelli editor/terminale)
  surface2: '#1c202b', // --surface-2 (widget di Monaco: suggerimenti, ricerca)
  txt: '#e9ecf5', // --txt
  dim: '#9aa3b8', // --dim
  faint: '#667089', // --faint
  accent: '#8fa8ff', // --accent
  work: '#5ee0c1', // --work
  wait: '#ffb454', // --wait
  err: '#ff8f8f', // --err
} as const

/** `#rrggbb` + alpha 0..1 → `#rrggbbaa` (Monaco accetta solo esadecimali). */
export function withAlpha(hex: string, alpha: number): string {
  const a = Math.round(Math.min(1, Math.max(0, alpha)) * 255)
  return hex + a.toString(16).padStart(2, '0')
}

const transparent = '#00000000'

// Sfondo trasparente: lo dà il pannello (.surface-opaque = --surface-1), come per ogni altra colonna.
export const monacoColors: Record<string, string> = {
  'editor.background': transparent,
  'editorGutter.background': transparent,
  'minimap.background': transparent,
  'editor.foreground': palette.txt,
  'editorLineNumber.foreground': palette.faint,
  'editorLineNumber.activeForeground': palette.dim,
  'editor.lineHighlightBackground': withAlpha('#ffffff', 0.05), // --glass
  'editor.selectionBackground': withAlpha(palette.accent, 0.25),
  'editorCursor.foreground': palette.accent,
  'editorWidget.background': palette.surface2,
  'editorWidget.border': withAlpha('#ffffff', 0.12), // --edge
  'editorSuggestWidget.background': palette.surface2,
  'editorSuggestWidget.border': withAlpha('#ffffff', 0.12), // --edge
  'editorHoverWidget.background': palette.surface2,
  'editorHoverWidget.border': withAlpha('#ffffff', 0.12), // --edge
  'scrollbarSlider.background': withAlpha('#ffffff', 0.1),
}

export const xtermTheme = {
  background: transparent, // come Monaco: lo sfondo è quello del pannello
  foreground: palette.txt,
  cursor: palette.accent,
  cursorAccent: palette.surface1,
  selectionBackground: withAlpha(palette.accent, 0.3), // --accent-line
  red: palette.err,
  green: palette.work,
  yellow: palette.wait,
  blue: palette.accent,
  brightBlack: palette.faint,
}
