const key = (sessionId: string) => `codeart.draft.${sessionId}`

/** Bozza non inviata della conversazione (solo testo). `localStorage` può mancare o lanciare: in tal caso nessuna bozza. */
export function loadDraft(sessionId: string): string {
  try {
    return localStorage.getItem(key(sessionId)) ?? ''
  } catch {
    return ''
  }
}

export function saveDraft(sessionId: string, text: string) {
  try {
    if (text) localStorage.setItem(key(sessionId), text)
    else localStorage.removeItem(key(sessionId))
  } catch {
    /* ignora */
  }
}

export const clearDraft = (sessionId: string) => saveDraft(sessionId, '')
