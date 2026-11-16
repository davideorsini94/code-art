/** Linguaggi con un language server: l'identificatore è quello di `lsp_start` (src-tauri/src/lsp/servers.rs). */
export type LspLang = 'python' | 'java' | 'go' | 'rust'

export const LSP_LANGS: readonly LspLang[] = ['python', 'java', 'go', 'rust']

const BY_EXT: Record<string, LspLang> = { py: 'python', java: 'java', go: 'go', rs: 'rust' }

export const LANG_LABEL: Record<LspLang, string> = { python: 'Python', java: 'Java', go: 'Go', rust: 'Rust' }

/** Linguaggio del file in base all'estensione (`.py`, `.java`, `.go`, `.rs`), o null. */
export function languageForPath(path: string): LspLang | null {
  const name = path.split('/').pop() ?? ''
  const dot = name.lastIndexOf('.')
  if (dot <= 0) return null
  return BY_EXT[name.slice(dot + 1).toLowerCase()] ?? null
}
