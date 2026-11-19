export type TerminalRequest = { kind: 'local'; cwd: string | undefined; seq: number } | { kind: 'remote'; serverId: string; dir: string; seq: number }

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
    this.requests = [...this.requests, { kind: 'local', cwd, seq: ++this.seq }]
  }

  /** Mostra il pannello e accoda un terminale su un server, nella cartella `dir` (assoluta o `~`). */
  openRemote(serverId: string, dir: string) {
    this.visible = true
    this.requests = [...this.requests, { kind: 'remote', serverId, dir, seq: ++this.seq }]
  }

  /** Restituisce e svuota la coda (la consuma TerminalPane). */
  take(): TerminalRequest[] {
    const out = this.requests
    this.requests = []
    return out
  }
}

export const terminals = new TerminalStore()
