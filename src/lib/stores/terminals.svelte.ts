export type TerminalRequest = { cwd: string | undefined; seq: number }

/** Punto d'ingresso unico per "apri un terminale in questa cartella". */
export class TerminalStore {
  visible = $state(false)
  requests = $state<TerminalRequest[]>([])
  private seq = 0

  toggle() {
    this.visible = !this.visible
  }

  show() {
    this.visible = true
  }

  /** Mostra il pannello e accoda la richiesta; senza `cwd` = cartella principale. */
  open(cwd?: string) {
    this.visible = true
    this.requests = [...this.requests, { cwd, seq: ++this.seq }]
  }

  /** Restituisce e svuota la coda (la consuma TerminalPane). */
  take(): TerminalRequest[] {
    const out = this.requests
    this.requests = []
    return out
  }
}

export const terminals = new TerminalStore()
