/**
 * Dati dei PTY arrivati prima che `pty_spawn` restituisse l'id (la scheda non sa ancora a chi appartengono).
 * I PTY chiusi (scheda chiusa o shell uscita) non accumulano più nulla: i loro dati tardivi si scartano.
 */
export class PtyInbox {
  private pending = new Map<number, Uint8Array[]>()
  private closed = new Set<number>()

  /** Tiene da parte i dati di un PTY non ancora collegato a una scheda (scartati se è già chiuso). */
  hold(id: number, bytes: Uint8Array) {
    if (this.closed.has(id)) return
    const list = this.pending.get(id)
    if (list) list.push(bytes)
    else this.pending.set(id, [bytes])
  }

  /** Consegna (e dimentica) i dati tenuti da parte per `id`. */
  take(id: number): Uint8Array[] {
    const list = this.pending.get(id) ?? []
    this.pending.delete(id)
    return list
  }

  /** Il PTY non c'è più: via i dati in attesa, d'ora in poi niente si accumula. */
  close(id: number) {
    this.closed.add(id)
    this.pending.delete(id)
  }

  /** PTY con dati in attesa. */
  get size(): number {
    return this.pending.size
  }
}
