<script lang="ts">
  import Icon from '../Icon.svelte'
  import { onMount } from 'svelte'
  import ChangeList from './ChangeList.svelte'
  import CommitBox from './CommitBox.svelte'
  import SyncBar from './SyncBar.svelte'
  import GitHubPanel from './GitHubPanel.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { git, stagedEntries, unstagedEntries } from '../../lib/stores/git.svelte'
  import { terminals } from '../../lib/stores/terminals.svelte'
  import { projectStore } from '../../lib/stores/project.svelte'

  const trusted = $derived(projectStore.untrusted.length === 0)
  const folders = $derived(projectStore.project?.folders ?? [])
  const norm = (p: string) => p.replace(/\/+$/, '')
  /** Cartelle del progetto senza alcun repository né in sé né sotto di sé: si possono inizializzare. */
  const nonRepos = $derived(git.loaded ? folders.filter((f) => !git.repos.some((r) => norm(r) === norm(f) || norm(r).startsWith(`${norm(f)}/`))) : [])
  /** Gruppi calcolati una volta per cambio di stato (non a ogni rendering). */
  const groups = $derived(Object.fromEntries(git.repos.map((r) => [r, git.status[r] ? { staged: stagedEntries(git.status[r]), unstaged: unstagedEntries(git.status[r]) } : null])))
  const name = (p: string) => p.replace(/\/+$/, '').split('/').pop() ?? p
  /** Titolo di una sezione: percorso relativo alla cartella del progetto che la contiene (`suis-be`, `apps/web`); la cartella stessa, per nome. */
  function title(repo: string) {
    const r = norm(repo)
    const f = folders.map(norm).find((f) => r.startsWith(`${f}/`))
    return f ? r.slice(f.length + 1) : name(repo)
  }
  /** Un solo repository, ed è la cartella del progetto: niente intestazione né chiusura. */
  const plain = $derived(git.repos.length === 1 && folders.some((f) => norm(f) === norm(git.repos[0])))
  /** Scelta dell'utente per sezione; senza scelta, aperta solo se ci sono modifiche. */
  let toggled = $state<Record<string, boolean>>({})
  const isOpen = (repo: string) => plain || (toggled[repo] ?? (git.status[repo]?.entries.length ?? 0) > 0)

  onMount(() => {
    void git.ensure()
  })
</script>

<div class="git">
  {#if !trusted}
    <div class="msg" role="status">
      Cartella non fidata: Git è disattivato finché non ti fidi.
      <button class="link" use:tooltip={'Considera fidata questa cartella: abilita modifiche, Claude e terminale'} onclick={() => void projectStore.acceptTrust(true)}>Mi fido</button>
    </div>
  {:else}
    {#if git.error}<div class="err" role="alert">{git.error}</div>{/if}
    {#if git.notice}<div class="note" role="status">{git.notice}</div>{/if}
    {#each git.repos as repo (repo)}
      {@const st = git.status[repo]}
      {@const g = groups[repo]}
      {#if st && g}
        {@const open = isOpen(repo)}
        <section class="repo" aria-label={title(repo)}>
          {#if !plain}
            <div class="head">
              <button class="toggle" aria-expanded={open} use:tooltip={open ? `Chiudi ${repo}` : `Apri ${repo}`} onclick={() => (toggled[repo] = !open)}>
                <span class="chev" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
                <span class="title">{title(repo)}</span>
              </button>
              <button class="branch" use:tooltip={'Cambia branch'} onclick={() => git.openPicker('switch', repo)}><Icon name="branch" size={13} /> {st.branch ?? (st.detached ? 'HEAD staccato' : 'nessun branch')}</button>
              {#if st.ahead > 0 || st.behind > 0}
                <span class="ab" use:tooltip={`${st.ahead} commit da inviare, ${st.behind} da scaricare`}><Icon name="arrowUp" size={12} />{st.ahead} <Icon name="arrowDown" size={12} />{st.behind}</span>
              {/if}
              {#if st.entries.length > 0}
                <span class="count" use:tooltip={`${st.entries.length} ${st.entries.length === 1 ? 'modifica' : 'modifiche'}`}>{st.entries.length}</span>
              {/if}
              <button class="term" aria-label={`Apri un terminale in ${title(repo)}`} use:tooltip={`Apri un terminale in ${title(repo)}`} onclick={() => terminals.open(repo)}><Icon name="terminal" size={14} /></button>
            </div>
          {/if}
          {#if open}
          {#if plain}
            <div class="syncrow">
              <div class="syncgrow"><SyncBar {repo} status={st} compact={false} /></div>
              <button class="term" aria-label={`Apri un terminale in ${title(repo)}`} use:tooltip={`Apri un terminale in ${title(repo)}`} onclick={() => terminals.open(repo)}><Icon name="terminal" size={14} /></button>
            </div>
          {:else}
            <SyncBar {repo} status={st} compact={true} />
          {/if}
          {#if st.merging}
            {@const n = st.entries.filter((e) => e.conflict).length}
            <div class="merge" role="status">
              <span>Unione in corso: {n} {n === 1 ? 'conflitto' : 'conflitti'}</span>
              <button class="link" disabled={git.busy !== null} use:tooltip={'Annulla l’unione e torna come prima'} onclick={() => void git.mergeAbort(repo)}>Annulla unione</button>
            </div>
          {/if}
          <CommitBox {repo} stagedCount={g.staged.length} />
          <ChangeList {repo} title="Modifiche in stage" entries={g.staged} staged={true} />
          <ChangeList {repo} title="Modifiche" entries={g.unstaged} staged={false} />
          {#if st.truncated}<div class="warn">Elenco troncato: ci sono troppe modifiche.</div>{/if}
          <GitHubPanel {repo} />
          {/if}
        </section>
      {/if}
    {/each}
    {#each nonRepos as f (f)}
      <div class="norepo">
        <div class="msg">{folders.length > 1 ? `${name(f)}: ` : ''}Questa cartella non è un repository Git.</div>
        <button class="act" aria-label={folders.length > 1 ? `Inizializza ${name(f)}` : 'Inizializza'} disabled={git.busy !== null} use:tooltip={'Crea un repository Git in questa cartella (git init)'} onclick={() => void git.init(f)}>Inizializza</button>
      </div>
    {/each}
  {/if}
</div>

<style>
  .git { display: flex; flex-direction: column; gap: 10px; padding: 8px; height: 100%; min-height: 0; overflow-y: auto; font-size: 12px; color: var(--txt); }
  .repo { display: flex; flex-direction: column; gap: 8px; }
  .head { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .toggle { display: flex; align-items: center; gap: 4px; flex: 1; min-width: 0; padding: 2px 4px; border: 0; border-radius: 6px; background: none; color: var(--txt); font: inherit; font-weight: 650; cursor: pointer; text-align: left; }
  .toggle:hover { background: var(--glass); }
  .chev { width: 10px; color: var(--dim); }
  .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .branch { max-width: 40%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 2px 8px; border-radius: 8px; border: 1px solid var(--edge-soft); background: var(--glass); color: var(--txt); font: inherit; font-size: 11.5px; font-family: ui-monospace, Menlo, monospace; cursor: pointer; }
  .branch:hover { background: var(--glass2); }
  .ab { font-size: 11.5px; color: var(--dim); font-variant-numeric: tabular-nums; }
  .count { padding: 0 6px; border-radius: 999px; background: var(--accent-soft); color: var(--txt); font-size: 10.5px; font-variant-numeric: tabular-nums; }
  @media (prefers-reduced-motion: no-preference) { .toggle, .branch { transition: background var(--dur); } }
  .syncrow { display: flex; align-items: center; gap: 6px; }
  .syncgrow { flex: 1; min-width: 0; }
  .term { display: inline-flex; align-items: center; justify-content: center; flex: none; padding: 4px; border: 0; border-radius: 6px; background: none; color: var(--dim); cursor: pointer; }
  .term:hover { background: var(--glass); color: var(--txt); }
  .msg { color: var(--dim); }
  .norepo { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
  .err { color: var(--err); padding: 4px 8px; border-radius: 8px; background: var(--err-soft); white-space: pre-wrap; }
  .note { color: var(--wait); }
  .warn { color: var(--wait); }
  .merge { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 4px 8px; border-radius: 8px; color: var(--wait); background: var(--wait-soft); border: 1px solid var(--wait-line); }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; text-decoration: underline; cursor: pointer; }
  .link:disabled { opacity: 0.45; cursor: not-allowed; }
  .act { padding: 4px 12px; border-radius: 8px; border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--txt); font: inherit; cursor: pointer; }
  .act:disabled { opacity: 0.45; cursor: not-allowed; }
  .branch, .ab { display: inline-flex; align-items: center; gap: 4px; }
  .ab { gap: 2px; }
  .chev { display: inline-flex; }
</style>
