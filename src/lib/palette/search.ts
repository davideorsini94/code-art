import { open } from '@tauri-apps/plugin-dialog'
import { projectAddFolder, projectPickAndOpen } from '../ipc'
import { conversations } from '../stores/conversations.svelte'
import { projectStore } from '../stores/project.svelte'

// ---------- ricerca fuzzy ----------

const SEP = new Set(['/', '.', '-', '_', ' '])

/** Minuscola carattere per carattere (1:1): gli indici restano allineati al testo originale. */
const fold = (s: string): string[] => Array.from(s, (c) => c.toLowerCase()[0] ?? c)

type Aligned = { score: number; pos: number[] }

/** Allineamento della sottosequenza a partire da `from` (prima occorrenza di ogni carattere). */
function alignFrom(q: string[], h: string[], from: number, nameStart: number): Aligned | null {
  const pos: number[] = []
  let i = from
  let score = 0
  for (const ch of q) {
    while (i < h.length && h[i] !== ch) i++
    if (i >= h.length) return null
    const prev = pos[pos.length - 1]
    score += 1
    if (prev !== undefined && prev === i - 1) score += 5 // contigua
    if (i === 0 || SEP.has(h[i - 1])) score += 3 // inizio di parola
    if (i >= nameStart) score += 4 // dentro il nome del file
    pos.push(i)
    i++
  }
  if (pos[0] >= nameStart) score += 6 // tutto nel nome del file
  return { score: score - h.length * 0.01, pos }
}

/** Migliore allineamento di `query` su `item` (provando ogni occorrenza del primo carattere), o null. */
function align(q: string[], item: string): Aligned | null {
  const h = fold(item)
  const nameStart = h.lastIndexOf('/') + 1
  let best: Aligned | null = null
  for (let s = 0; s < h.length; s++) {
    if (h[s] !== q[0]) continue
    const a = alignFrom(q, h, s, nameStart)
    if (a && (!best || a.score > best.score)) best = a
  }
  return best
}

/** Indici degli `items` che contengono `query` come sottosequenza, dal migliore (a parità, ordine originale). */
export function rankIndices(query: string, items: string[], limit: number): number[] {
  const q = fold(query.trim())
  if (q.length === 0) return items.slice(0, limit).map((_, i) => i)
  const scored: { i: number; score: number }[] = []
  items.forEach((it, i) => {
    const a = align(q, it)
    if (a) scored.push({ i, score: a.score })
  })
  scored.sort((a, b) => b.score - a.score || a.i - b.i)
  return scored.slice(0, limit).map((s) => s.i)
}

export function fuzzyRank(query: string, items: string[], limit: number): string[] {
  return rankIndices(query, items, limit).map((i) => items[i])
}

/** Posizioni (indici di carattere) della corrispondenza fuzzy di `query` in `text`, per l'evidenziazione. */
export function fuzzyPositions(query: string, text: string): number[] {
  const q = fold(query.trim())
  return q.length ? (align(q, text)?.pos ?? []) : []
}

/** Posizioni della prima occorrenza di `query` come sottostringa (senza maiuscole). */
export function substringPositions(query: string, text: string): number[] {
  const q = fold(query.trim())
  const h = fold(text)
  if (!q.length) return []
  for (let i = 0; i + q.length <= h.length; i++) {
    if (h[i] === q[0] && q.every((c, k) => h[i + k] === c)) return q.map((_, k) => i + k)
  }
  return []
}

/** Spezza il testo in pezzi evidenziati e non: si mostrano come testo, mai come HTML. */
export function segments(text: string, positions: number[]): { t: string; hit: boolean }[] {
  const on = new Set(positions)
  const out: { t: string; hit: boolean }[] = []
  Array.from(text).forEach((c, i) => {
    const hit = on.has(i)
    const last = out[out.length - 1]
    if (last && last.hit === hit) last.t += c
    else out.push({ t: c, hit })
  })
  return out
}

/** Percorso assoluto di un risultato di `fs_all_files`; con più cartelle il primo segmento è il nome della cartella. */
export function absolutePath(folders: string[], rel: string): string {
  if (folders.length <= 1) return `${(folders[0] ?? '').replace(/\/+$/, '')}/${rel}`
  const i = rel.indexOf('/')
  const head = rel.slice(0, i)
  const folder = folders.find((f) => f.replace(/\/+$/, '').split('/').pop() === head) ?? folders[0]
  return `${folder.replace(/\/+$/, '')}/${rel.slice(i + 1)}`
}

// ---------- ricerca più recente ----------

/** Esegue `run` dopo `delay` ms dall'ultima `query`; consegna solo il risultato dell'ultima interrogazione. */
export function createLatest<T>(run: (q: string) => Promise<T>, onResult: (q: string, r: T) => void, delay: number) {
  let seq = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  const cancel = () => {
    seq++
    if (timer) clearTimeout(timer)
    timer = undefined
  }
  const query = (q: string) => {
    cancel()
    const mine = seq
    timer = setTimeout(() => {
      timer = undefined
      run(q).then(
        (r) => {
          if (mine === seq) onResult(q, r)
        },
        () => {},
      )
    }, delay)
  }
  return { query, cancel }
}

// ---------- comandi ----------

/** Azioni che dipendono da componenti: le registrano App, Titlebar e Palette al montaggio. */
export const hooks = { toggleTerminal: () => {}, renameProject: () => {}, openModes: () => {} }

export const PERMISSION_MODES: [mode: string, label: string][] = [
  ['bypassPermissions', 'Senza conferme'],
  ['acceptEdits', 'Accetta modifiche'],
  ['plan', 'Piano'],
  ['default', 'Chiedi sempre'],
]

export type Command = { id: string; title: string; run(): void }

export const commands: Command[] = [
  { id: 'conv.new', title: 'Nuova conversazione', run: () => void conversations.create() },
  { id: 'project.open', title: 'Apri cartella', run: () => void projectPickAndOpen() },
  {
    id: 'project.addFolder',
    title: 'Aggiungi cartella al progetto',
    run: () =>
      void (async () => {
        const project = projectStore.project
        if (!project) return
        const picked = await open({ directory: true, multiple: false, title: 'Aggiungi cartella al progetto' })
        if (typeof picked === 'string') projectStore.project = await projectAddFolder(project.id, picked)
      })().catch(() => {}),
  },
  { id: 'project.rename', title: 'Rinomina progetto', run: () => hooks.renameProject() },
  { id: 'terminal.toggle', title: 'Mostra/nascondi terminale', run: () => hooks.toggleTerminal() },
  {
    id: 'conv.archive',
    title: 'Archivia conversazione',
    run: () => {
      const id = conversations.activeId
      if (id) void conversations.archive(id, true)
    },
  },
  { id: 'settings.permissionMode', title: 'Cambia modalità permessi predefinita', run: () => hooks.openModes() },
]
