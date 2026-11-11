// Limiti sui caricamenti di immagini innescati da un documento markdown (che può essere ostile):
// poche richieste alla volta, poche immagini, tetto per immagine e in totale.
export type Limits = { concurrency: number; maxImages: number; perImage: number; total: number }

export const IMAGE_LIMITS: Limits = { concurrency: 4, maxImages: 100, perImage: 10 * 1024 * 1024, total: 60 * 1024 * 1024 }

export type Loaded = { url: string; size: number }
type Load = (key: string, maxBytes: number) => Promise<Loaded>

/** Coda di caricamenti con budget. `request` risolve con l'URL o `null` (oltre i limiti, errore, già smontato). */
export class ImageLoader {
  private results = new Map<string, Promise<string | null>>()
  private waiting: (() => void)[] = []
  private running = 0
  private used = 0
  private disposed = false
  private urls: string[] = []

  constructor(
    private load: Load,
    private revoke: (url: string) => void,
    private limits: Limits = IMAGE_LIMITS,
  ) {}

  request(key: string): Promise<string | null> {
    const known = this.results.get(key)
    if (known) return known
    if (this.disposed || this.results.size >= this.limits.maxImages) return Promise.resolve(null)
    const p = this.run(key)
    this.results.set(key, p)
    return p
  }

  private async run(key: string): Promise<string | null> {
    if (this.running >= this.limits.concurrency) await new Promise<void>((r) => this.waiting.push(r))
    this.running++
    try {
      const room = this.limits.total - this.used
      if (this.disposed || room <= 0) return null
      const r = await this.load(key, Math.min(this.limits.perImage, room))
      if (this.disposed) {
        this.revoke(r.url)
        return null
      }
      this.used += r.size
      this.urls.push(r.url)
      return r.url
    } catch {
      return null
    } finally {
      this.running--
      this.waiting.shift()?.()
    }
  }

  dispose() {
    this.disposed = true
    for (const u of this.urls) this.revoke(u)
    this.urls = []
    for (const w of this.waiting.splice(0)) w()
  }
}
