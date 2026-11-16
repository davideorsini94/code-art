import * as ipc from '../ipc'
import { errorText } from '../errors'
import { openExternal } from '../external'
import { git } from './git.svelte'
import type { ChecksSummary, GhAccount, MergeMethod, Pr, PrCheck, PrState } from '../types'

/** Errore del core quando `gh` non è installato. */
export const GH_MISSING = 'GitHub CLI (`gh`) non trovata: `brew install gh`'
export const ACCOUNT_KEY = 'github_account'

export const MERGE_LABEL: Record<MergeMethod, string> = { merge: 'Commit di unione', squash: 'Squash', rebase: 'Rebase' }
export const STATE_LABEL: Record<PrState, string> = { open: 'Aperte', closed: 'Chiuse', all: 'Tutte' }

/** Colore del pallino dei controlli: rosso se uno fallisce, ambra se qualcuno è in corso, verde se tutti passano. */
export type CheckColor = 'ok' | 'fail' | 'wait' | 'none'
export function checkColor(c: ChecksSummary): CheckColor {
  if (c.failing > 0) return 'fail'
  if (c.pending > 0) return 'wait'
  if (c.passing > 0) return 'ok'
  return 'none'
}

export function checksText(c: ChecksSummary): string {
  if (c.passing + c.failing + c.pending === 0) return 'Nessun controllo'
  return `Controlli: ${c.passing} superati, ${c.failing} falliti, ${c.pending} in corso`
}

/** GitHub nella vista Git: account del progetto, remoto GitHub e pull request per repository.
 *  Errori ed esiti passano da `git.error`/`git.notice` (pannello Git e GitToast li mostrano già). */
export class GitHubStore {
  accounts = $state<GhAccount[]>([])
  /** Account scelto per il progetto ('' = account attivo di `gh`). */
  account = $state('')
  /** `gh` non installato. */
  missing = $state(false)
  /** Repository → `proprietario/nome` su GitHub; `null` = nessun `origin` (si può pubblicare); `''` = `origin` non su
   *  GitHub. Assente = non ancora letto. */
  remotes = $state<Record<string, string | null>>({})
  prs = $state<Record<string, Pr[]>>({})
  prState = $state<PrState>('open')
  checks = $state<Record<string, PrCheck[]>>({})
  loading = $state(false)
  busy = $state<'publish' | 'create' | 'merge' | null>(null)
  /** Repository con il modulo "Nuova pull request" aperto. */
  creating = $state<string | null>(null)
  /** Finestra "Clona da GitHub…" aperta. */
  cloneOpen = $state(false)

  private accountsLoaded = false

  get activeLogin(): string | null {
    return this.accounts.find((a) => a.active)?.login ?? null
  }

  private fail(e: unknown) {
    if (String(e) === GH_MISSING) this.missing = true
    else git.error = errorText(e)
  }

  /** Account di `gh` e account scelto per il progetto (una volta; di nuovo se `gh` mancava). */
  async init() {
    if (this.accountsLoaded && !this.missing) return
    try {
      this.accounts = await ipc.ghAccounts()
      this.missing = false
      this.accountsLoaded = true
    } catch (e) {
      this.fail(e)
      return
    }
    this.account = (await ipc.projectSettingGet(ACCOUNT_KEY).catch(() => null)) ?? ''
  }

  /** Rilegge il remoto GitHub del repository e, se c'è, le sue pull request. */
  async refreshRepo(repo: string) {
    let nwo: string | null
    try {
      const r = await ipc.ghRemote(repo)
      nwo = r.github ?? (r.origin ? '' : null)
    } catch (e) {
      this.fail(e)
      return
    }
    this.remotes = { ...this.remotes, [repo]: nwo }
    if (nwo && !this.missing) await this.refreshPrs(repo)
  }

  async refreshPrs(repo: string) {
    this.loading = true
    try {
      const list = await ipc.ghPrList(repo, this.prState)
      this.prs = { ...this.prs, [repo]: list }
    } catch (e) {
      this.fail(e)
    } finally {
      this.loading = false
    }
  }

  private refreshAll() {
    return Promise.all(Object.keys(this.remotes).filter((r) => this.remotes[r]).map((r) => this.refreshPrs(r)))
  }

  async setAccount(login: string) {
    try {
      await ipc.projectSettingSet(ACCOUNT_KEY, login)
    } catch (e) {
      this.fail(e)
      return
    }
    this.account = login
    await this.refreshAll()
  }

  async setPrState(state: PrState) {
    this.prState = state
    await this.refreshAll()
  }

  async loadChecks(repo: string, number: number) {
    try {
      const list = await ipc.ghPrChecks(repo, number)
      this.checks = { ...this.checks, [`${repo}#${number}`]: list }
    } catch (e) {
      this.fail(e)
    }
  }

  /** Esegue un'operazione GitHub: una alla volta, errore azzerato al successo. */
  private async act<T>(op: 'publish' | 'create' | 'merge', fn: () => Promise<T>): Promise<T | undefined> {
    if (this.busy) return undefined
    this.busy = op
    git.notice = null
    try {
      const r = await fn()
      git.error = null
      return r
    } catch (e) {
      this.fail(e)
      return undefined
    } finally {
      this.busy = null
    }
  }

  /** "Pubblica su GitHub": crea il repository, aggiunge `origin` e invia. */
  async publish(repo: string, name: string, priv: boolean, description: string): Promise<boolean> {
    const url = await this.act('publish', () => ipc.ghRepoCreate(repo, name, priv, description))
    if (url === undefined) return false
    git.notice = `Pubblicato su GitHub: ${url}`
    void git.refresh()
    await this.refreshRepo(repo)
    return true
  }

  async createPr(repo: string, title: string, body: string, base: string, draft: boolean): Promise<Pr | undefined> {
    const pr = await this.act('create', () => ipc.ghPrCreate(repo, title, body, base, draft))
    if (!pr) return undefined
    git.notice = `Pull request #${pr.number} creata`
    this.creating = null
    void git.refresh()
    await this.refreshPrs(repo)
    return pr
  }

  /** Unisce dopo la conferma nativa del core; `true` se unita. */
  async merge(repo: string, number: number, method: MergeMethod, deleteBranch: boolean): Promise<boolean> {
    const ok = await this.act('merge', () => ipc.ghPrMerge(repo, number, method, deleteBranch))
    if (!ok) return false
    git.notice = `Pull request #${number} unita`
    void git.refresh()
    await this.refreshPrs(repo)
    return true
  }

  /** Link nel browser di sistema. */
  open(url: string) {
    openExternal(url)
  }
}

export const github = new GitHubStore()
