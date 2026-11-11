import type { Conversation } from '../types'

/** Modalità permessi della CLI con la loro etichetta (compositore e palette). */
export const PERMISSION_MODES: [mode: string, label: string][] = [
  ['bypassPermissions', 'Senza conferme'],
  ['acceptEdits', 'Accetta modifiche'],
  ['plan', 'Piano'],
  ['default', 'Chiedi sempre'],
]

/** Spiegazione dello stato di una conversazione (pallino della scheda e dell'intestazione). */
export const STATUS_TIP: Record<Conversation['status'], string> = {
  waiting: 'Aspetta una tua risposta',
  working: 'Claude sta lavorando',
  idle: 'Nessuna attività in corso',
}

/** Perché i controlli che avviano Claude sono spenti in una cartella non fidata. */
export const UNTRUSTED_HINT = 'Cartella non fidata — sola lettura'
