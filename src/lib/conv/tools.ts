/** Aiuti per mostrare gli strumenti di Claude. Tutto restituisce testo semplice: mai HTML. */

export const RESULT_CAP = 20_000

export const isSubagent = (name: string) => name === 'Agent' || name === 'Task'
export const isEditTool = (name: string) => name === 'Edit' || name === 'Write' || name === 'MultiEdit'

const str = (o: unknown, k: string): string | null => {
  if (!o || typeof o !== 'object') return null
  const v = (o as Record<string, unknown>)[k]
  return typeof v === 'string' && v ? v : null
}

/** Argomento principale dello strumento in poche parole (percorso, comando, descrizione…). */
export function toolSummary(name: string, input: unknown): string {
  const keys = isSubagent(name) ? ['description', 'prompt'] : ['file_path', 'notebook_path', 'command', 'path', 'url', 'pattern', 'query', 'description']
  for (const k of keys) {
    const v = str(input, k)
    if (v) return v.length > 140 ? v.slice(0, 140) + '…' : v
  }
  return ''
}

export type DiffAction = { type: 'diff'; path: string; before: string; after: string } | { type: 'open'; path: string }

/**
 * Cosa fa "Apri diff". `Edit` e `MultiEdit` portano le stringhe vecchie/nuove: il diff mostra quei frammenti
 * (non l'intero file, che l'input non contiene). `Write` ha solo il contenuto nuovo: nessun "prima" fedele,
 * quindi si apre il file così com'è ora.
 */
export function diffAction(name: string, input: unknown, filePath: string | null): DiffAction | null {
  const path = filePath ?? str(input, 'file_path')
  if (!path || !isEditTool(name)) return null
  if (name === 'Edit') {
    const before = str(input, 'old_string')
    const after = typeof (input as Record<string, unknown>)?.new_string === 'string' ? ((input as Record<string, unknown>).new_string as string) : null
    if (before !== null && after !== null) return { type: 'diff', path, before, after }
  }
  if (name === 'MultiEdit') {
    const edits = (input as { edits?: unknown })?.edits
    if (Array.isArray(edits) && edits.length > 0 && edits.every((e) => typeof e?.old_string === 'string' && typeof e?.new_string === 'string')) {
      return {
        type: 'diff',
        path,
        before: edits.map((e) => e.old_string as string).join('\n'),
        after: edits.map((e) => e.new_string as string).join('\n'),
      }
    }
  }
  return { type: 'open', path }
}

export function prettyJson(v: unknown): string {
  try {
    return JSON.stringify(v, null, 2) ?? ''
  } catch {
    return String(v)
  }
}

/** Primi `limit` caratteri; `truncated` dice se c'era di più. */
export function capText(s: string, limit = RESULT_CAP): { text: string; truncated: boolean } {
  return s.length > limit ? { text: s.slice(0, limit), truncated: true } : { text: s, truncated: false }
}
