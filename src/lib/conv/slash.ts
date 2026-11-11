/** Comandi offerti finché la conversazione non ha un `init` (processo non ancora avviato in questa esecuzione). */
export const BUILTIN_COMMANDS = ['compact', 'clear', 'model', 'help']

const MAX = 8
const ACTIVE = /^\/([A-Za-z][\w:.-]*)?$/

/**
 * Completamento dei comandi `/nome`: attivo solo se il testo prima del cursore è `/` + lettere all'inizio del messaggio.
 * Prima i comandi che iniziano con il testo digitato, poi quelli che lo contengono; al massimo 8.
 */
export function slashMatches(input: string, cursor: number, commands: string[]): string[] {
  const m = ACTIVE.exec(input.slice(0, cursor))
  if (!m) return []
  const q = (m[1] ?? '').toLowerCase()
  const uniq = [...new Set(commands)]
  const prefix = uniq.filter((c) => c.toLowerCase().startsWith(q))
  const rest = uniq.filter((c) => !c.toLowerCase().startsWith(q) && c.toLowerCase().includes(q))
  return [...prefix, ...rest].slice(0, MAX)
}

/** Sostituisce la parola `/xyz` prima del cursore con `/comando ` e dà il nuovo testo e la nuova posizione del cursore. */
export function completeSlash(input: string, cursor: number, command: string): { text: string; cursor: number } {
  const head = `/${command} `
  return { text: head + input.slice(cursor), cursor: head.length }
}
