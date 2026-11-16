import type { LspLang } from '../lsp/languages'
import type { LspStatus } from '../lsp/monaco-bridge'

/** Stato dei language server per l'interfaccia (indicatore nell'editor e riga informativa). Vive fuori dal modulo
 *  caricato al primo file, così l'interfaccia lo legge anche prima. */
class LspStore {
  status = $state<Partial<Record<LspLang, { state: LspStatus; detail?: string }>>>({})
  /** Riga informativa (server mancante, progetto non fidato), una volta per sessione e linguaggio. */
  notice = $state<string | null>(null)

  setStatus(lang: LspLang, state: LspStatus, detail?: string) {
    this.status[lang] = { state, detail }
  }
}

export const lspStore = new LspStore()
