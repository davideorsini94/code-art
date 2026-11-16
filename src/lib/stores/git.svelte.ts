import type { UnlistenFn } from '@tauri-apps/api/event'
import { listenProject } from '../events'
import * as ipc from '../ipc'
import { errorText } from '../errors'
import { editor } from './editor.svelte'
import { projectStore } from './project.svelte'
import type { GitChangedEvent, GitDiffText, GitOpDoneEvent, GitProgressEvent, GitRemoteOp, GitRepoStatus, GitStatusEntry, MergeOutcome } from '../types'

export type GitBusy = { repo: string; op: string; id: number | null; line: string }
export type PickerMode = 'switch' | 'create' | 'merge'

/** Oltre il tempo massimo del core (10 min) un'operazione remota senza `git:op_done` si considera finita. */
export const REMOTE_GUARD_MS = 11 * 60 * 1000

export const REMOTE_LABEL: Record<GitRemoteOp, string> = { fetch: 'Fetch', pull: 'Pull', push: 'Push' }

/** Voci già in stage (i conflitti restano tra le modifiche). */
export const stagedEntries = (s: GitRepoStatus) => s.entries.filter((e) => e.staged && !e.conflict)
/** Voci non in stage: modificate, non tracciate o in conflitto. */
export const unstagedEntries = (s: GitRepoStatus) => s.entries.filter((e) => e.conflict || e.untracked || e.unstaged)

const join = (repo: string, path: string) => `${repo.replace(/\/+$/, '')}/${path}`

/** Stato Git della finestra: repository del progetto, stato per repository, operazione in corso e ultimo errore. */
export class GitStore {
  repos = $state<string[]>([])
  status = $state<Record<string, GitRepoStatus>>({})
  busy = $state<GitBusy | null>(null)
  error = $state<string | null>(null)
  /** Ultimo esito di unione con conflitti, per il messaggio. */
  notice = $state<string | null>(null)
  /** Selettore di branch aperto (titolo e palette lo aprono da qui). */
  picker = $state<{ repo: string; mode: PickerMode } | null>(null)
  /** Incrementa per riportare il focus sul campo del messaggio di commit. */
  focusTick = $state(0)
  loaded = $state(false)

  private unlisten: UnlistenFn[] = []
  private boundId: string | null = null
  private gen = 0
  private onFocus = () => void this.refresh()
  /** Eventi di un'operazione remota arrivati prima che `git_remote_op` rispondesse con l'id. */
  private early: (['progress', GitProgressEvent] | ['done', GitOpDoneEvent])[] = []
  private starting = false
  private seq = 0
  private refreshFailed = false
  private guard: ReturnType<typeof setTimeout> | null = null

  get primary(): string | null {
    return this.repos[0] ?? null
  }

  /** Si mette in ascolto degli eventi git di questo progetto (idempotente) e carica lo stato. */
  async bind(projectId: string) {
    if (this.boundId === projectId) return
    this.unbind()
    this.boundId = projectId
    const gen = ++this.gen
    const subs = await Promise.all([
      listenProject<GitChangedEvent>('git:changed', projectId, () => void this.refresh()),
      listenProject<GitProgressEvent>('git:progress', projectId, (e) => this.onRemote(['progress', e.payload])),
      listenProject<GitOpDoneEvent>('git:op_done', projectId, (e) => this.onRemote(['done', e.payload])),
    ])
    if (gen !== this.gen) {
      subs.forEach((u) => u())
      return
    }
    this.unlisten = subs
    window.addEventListener('focus', this.onFocus)
    await this.refresh()
  }

  unbind() {
    this.boundId = null
    this.gen++
    this.unlisten.forEach((u) => u())
    this.unlisten = []
    window.removeEventListener('focus', this.onFocus)
    this.clearGuard()
    this.busy = null
  }

  private clearGuard() {
    if (this.guard) clearTimeout(this.guard)
    this.guard = null
  }

  /** Assicura che lo store sia collegato al progetto della finestra (per titolo e palette). */
  async ensure() {
    const id = projectStore.project?.id
    if (id) await this.bind(id)
  }

  /** Rilegge repository e stato. Un progetto non fidato non esegue git: niente richieste. */
  async refresh() {
    if (!projectStore.project || projectStore.untrusted.length > 0) return
    const my = ++this.seq
    try {
      const repos = await ipc.gitRepos()
      const list = await Promise.all(repos.map((r) => ipc.gitStatus(r)))
      if (my !== this.seq) return
      this.repos = repos
      this.status = Object.fromEntries(repos.map((r, i) => [r, list[i]]))
      this.loaded = true
      if (this.refreshFailed) {
        this.refreshFailed = false
        this.error = null
      }
    } catch (e) {
      if (my !== this.seq) return
      this.refreshFailed = true
      this.error = errorText(e)
    }
  }

  private fail(e: unknown) {
    this.refreshFailed = false
    this.error = errorText(e)
    return undefined
  }

  /** Esegue un'operazione locale: una alla volta, errore azzerato al primo successo. */
  private async local<T>(repo: string, op: string, fn: () => Promise<T>): Promise<T | undefined> {
    if (this.busy) return undefined
    this.busy = { repo, op, id: null, line: '' }
    this.notice = null
    try {
      const r = await fn()
      this.error = null
      return r
    } catch (e) {
      return this.fail(e)
    } finally {
      this.busy = null
      void this.refresh()
    }
  }

  stage(repo: string, paths: string[]) {
    return this.local(repo, 'stage', () => ipc.gitStage(repo, paths))
  }

  unstage(repo: string, paths: string[]) {
    return this.local(repo, 'unstage', () => ipc.gitUnstage(repo, paths))
  }

  /** Scarta le modifiche non in stage (conferma nativa del core). */
  discard(repo: string, paths: string[]) {
    return this.local(repo, 'discard', () => ipc.gitDiscard(repo, paths))
  }

  /** `true` se il commit è stato creato. */
  async commit(repo: string, message: string, amend: boolean): Promise<boolean> {
    const hash = await this.local(repo, 'commit', () => ipc.gitCommit(repo, message, amend))
    return hash !== undefined
  }

  async checkout(repo: string, branch: string): Promise<boolean> {
    const ok = await this.local(repo, 'checkout', async () => (await ipc.gitCheckout(repo, branch), true))
    if (ok) void projectStore.refreshBranch()
    return !!ok
  }

  async createBranch(repo: string, name: string, from: string | null): Promise<boolean> {
    const ok = await this.local(repo, 'branch', async () => (await ipc.gitCreateBranch(repo, name, from, true), true))
    if (ok) void projectStore.refreshBranch()
    return !!ok
  }

  async merge(repo: string, branch: string): Promise<MergeOutcome | undefined> {
    const out = await this.local(repo, 'merge', () => ipc.gitMerge(repo, branch))
    if (out) {
      const n = out.conflicts.length
      this.notice =
        out.kind === 'conflicts' ? `Unione con ${n} ${n === 1 ? 'conflitto' : 'conflitti'}: risolvili o annulla l’unione`
        : out.kind === 'up_to_date' ? `${branch} è già incluso nel branch corrente`
        : `${branch} unito nel branch corrente`
    }
    return out
  }

  mergeAbort(repo: string) {
    return this.local(repo, 'abort', () => ipc.gitMergeAbort(repo))
  }

  init(folder: string) {
    return this.local(folder, 'init', () => ipc.gitInit(folder))
  }

  /** Apre il diff di una voce nell'editor (sola lettura; per i file binari solo un avviso). Un file in conflitto
   *  non ha un confronto con l'indice: si apre il file, dove compaiono i pulsanti di risoluzione. */
  async openDiff(repo: string, e: GitStatusEntry, staged: boolean) {
    if (e.conflict && !staged) {
      this.error = null
      void editor.open(join(repo, e.path))
      return
    }
    let d: GitDiffText
    try {
      d = await ipc.gitDiff(repo, e.path, e.origPath, staged)
    } catch (err) {
      this.fail(err)
      return
    }
    if (d.binary) {
      this.error = 'File binario o troppo grande: nessun confronto da mostrare'
      return
    }
    this.error = null
    // il lato in cartella di lavoro si modifica solo per le modifiche non in stage; l'indice resta in sola lettura
    void editor.openDiff(join(repo, e.path), d.before, d.after, { editable: !staged })
  }

  /** Percorso assoluto di una voce, per "apri file". */
  absolute(repo: string, path: string) {
    return join(repo, path)
  }

  // ---------- operazioni remote ----------

  /** Fetch/pull/push: una sola operazione per volta; avanzamento su `busy.line`, fine su `git:op_done`. */
  async remote(repo: string, op: GitRemoteOp) {
    if (this.busy) return
    this.busy = { repo, op, id: null, line: '' }
    this.starting = true
    this.early = []
    this.notice = null
    try {
      const id = await ipc.gitRemoteOp(repo, op)
      this.starting = false
      if (this.busy) this.busy = { ...this.busy, id }
      // rete di sicurezza: se `git:op_done` non arriva (finestra ricaricata, evento perso) lo stato non resta bloccato
      this.clearGuard()
      this.guard = setTimeout(() => {
        this.guard = null
        if (this.busy?.id === id) this.busy = null
      }, REMOTE_GUARD_MS)
      const pending = this.early
      this.early = []
      pending.forEach((e) => this.onRemote(e))
    } catch (e) {
      this.starting = false
      this.busy = null
      this.clearGuard()
      this.fail(e)
    }
  }

  /** Gli eventi possono arrivare prima della risposta di `git_remote_op`: si tengono da parte. */
  private onRemote(e: ['progress', GitProgressEvent] | ['done', GitOpDoneEvent]) {
    if (this.starting) {
      this.early.push(e)
      return
    }
    const b = this.busy
    if (!b || b.id !== e[1].id) return
    if (e[0] === 'progress') {
      this.busy = { ...b, line: e[1].line }
      return
    }
    this.busy = null
    this.clearGuard()
    this.error = e[1].ok ? null : e[1].message
    this.notice = e[1].ok ? `${REMOTE_LABEL[b.op as GitRemoteOp] ?? b.op} completato` : null
    void this.refresh()
    if (e[1].ok) void projectStore.refreshBranch()
  }

  async cancel() {
    const id = this.busy?.id
    if (id == null) return
    await ipc.gitCancel(id).catch(() => {})
  }

  // ---------- selettore di branch ----------

  openPicker(mode: PickerMode, repo = this.primary) {
    if (repo) this.picker = { repo, mode }
  }

  closePicker() {
    this.picker = null
  }
}

export const git = new GitStore()

/** Errore leggibile se `name` non è un nome di branch valido (regole di `git check-ref-format`, in sintesi); `null` se va bene. */
export function branchNameError(name: string): string | null {
  if (name === '') return 'Scrivi un nome'
  if (name.startsWith('-')) return 'Il nome non può iniziare con «-»'
  if (/[\s~^:?*[\\\x00-\x1f\x7f]/.test(name)) return 'Spazi e i caratteri ~ ^ : ? * [ \\ non sono ammessi'
  if (name.includes('..') || name.includes('@{') || name.includes('//')) return 'Il nome contiene una sequenza non ammessa (.., @{ o //)'
  if (name === '@') return 'Il nome non può essere «@»'
  if (name.startsWith('/') || name.endsWith('/')) return 'Il nome non può iniziare o finire con «/»'
  if (name.endsWith('.')) return 'Il nome non può finire con un punto'
  if (name.split('/').some((p) => p.startsWith('.') || p.endsWith('.lock'))) return 'Nessuna parte può iniziare con «.» o finire con «.lock»'
  return null
}
