import type { AskQuestion } from '../types'

/** Scelte dell'utente per una domanda: opzioni selezionate e testo libero ("Altro"). */
export type Picked = { options: string[]; other: string }

/** Risposte per `conv_answer` (oggetto `{ domanda: risposta | [risposte] }`), o null se manca una risposta. */
export function multiAnswer(questions: AskQuestion[], picked: Picked[]): Record<string, string | string[]> | null {
  if (questions.length === 0 || picked.length !== questions.length) return null
  const out: Record<string, string | string[]> = {}
  for (const [i, q] of questions.entries()) {
    const other = picked[i].other.trim()
    if (q.multiSelect) {
      const all = [...picked[i].options, ...(other ? [other] : [])]
      if (!all.length) return null
      out[q.question] = all
    } else {
      const one = other || picked[i].options[0]
      if (!one) return null
      out[q.question] = one
    }
  }
  return out
}
