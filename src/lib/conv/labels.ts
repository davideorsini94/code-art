import type { Conversation } from '../types'

/** Modalità permessi della CLI con la loro etichetta (compositore e palette). */
/** Livelli di thinking di `claude --effort` (gli stessi valori accettati dal core). */
export const EFFORTS: [effort: string, label: string][] = [
  ['low', 'Basso'],
  ['medium', 'Medio'],
  ['high', 'Alto'],
  ['xhigh', 'Molto alto'],
  ['max', 'Massimo'],
]

export const PERMISSION_MODES: [mode: string, label: string][] = [
  ['bypassPermissions', 'Senza conferme'],
  ['acceptEdits', 'Accetta modifiche'],
  ['plan', 'Piano'],
  ['default', 'Chiedi sempre'],
]

/** Spiegazione dello stato di una conversazione (pallino della scheda e dell'intestazione). */
/** Titolo da mostrare: una conversazione nuova non ha ancora un titolo (arriva dopo il primo turno). */
export const convTitle = (title: string) => title.trim() || 'Nuova conversazione'

export const STATUS_TIP: Record<Conversation['status'], string> = {
  waiting: 'Aspetta una tua risposta',
  working: 'Claude sta lavorando',
  idle: 'Nessuna attività in corso',
}

/** Perché i controlli che avviano Claude sono spenti in una cartella non fidata. */
export const UNTRUSTED_HINT = 'Cartella non fidata — sola lettura'
