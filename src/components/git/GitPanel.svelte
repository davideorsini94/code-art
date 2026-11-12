<script lang="ts">
  import { onMount } from 'svelte'
  import ChangeList from './ChangeList.svelte'
  import CommitBox from './CommitBox.svelte'
  import SyncBar from './SyncBar.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { git, stagedEntries, unstagedEntries } from '../../lib/stores/git.svelte'
  import { projectStore } from '../../lib/stores/project.svelte'

  const trusted = $derived(projectStore.untrusted.length === 0)
  const folders = $derived(projectStore.project?.folders ?? [])
  const name = (p: string) => p.replace(/\/+$/, '').split('/').pop() ?? p

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
    {#if git.loaded && git.repos.length === 0}
      {#each folders as f (f)}
        <div class="norepo">
          <div class="msg">{folders.length > 1 ? `${name(f)}: ` : ''}Questa cartella non è un repository Git.</div>
          <button class="act" disabled={git.busy !== null} use:tooltip={'Crea un repository Git in questa cartella (git init)'} onclick={() => void git.init(f)}>Inizializza</button>
        </div>
      {/each}
    {/if}
    {#each git.repos as repo (repo)}
      {@const st = git.status[repo]}
      {#if st}
        <section class="repo" aria-label={name(repo)}>
          {#if git.repos.length > 1}<h3 use:tooltip={repo}>{name(repo)}</h3>{/if}
          <SyncBar {repo} status={st} />
          {#if st.merging}
            {@const n = st.entries.filter((e) => e.conflict).length}
            <div class="merge" role="status">
              <span>Unione in corso: {n} {n === 1 ? 'conflitto' : 'conflitti'}</span>
              <button class="link" disabled={git.busy !== null} use:tooltip={'Annulla l’unione e torna come prima'} onclick={() => void git.mergeAbort(repo)}>Annulla unione</button>
            </div>
          {/if}
          <CommitBox {repo} stagedCount={stagedEntries(st).length} />
          <ChangeList {repo} title="Modifiche in stage" entries={stagedEntries(st)} staged={true} />
          <ChangeList {repo} title="Modifiche" entries={unstagedEntries(st)} staged={false} />
          {#if st.truncated}<div class="warn">Elenco troncato: ci sono troppe modifiche.</div>{/if}
        </section>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .git { display: flex; flex-direction: column; gap: 10px; padding: 8px; height: 100%; min-height: 0; overflow-y: auto; font-size: 12px; color: var(--txt); }
  .repo { display: flex; flex-direction: column; gap: 8px; }
  h3 { margin: 0; font-size: 12px; font-weight: 650; }
  .msg { color: var(--dim); }
  .norepo { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
  .err { color: var(--err); padding: 4px 8px; border-radius: 8px; background: var(--err-soft); white-space: pre-wrap; }
  .warn { color: var(--wait); }
  .merge { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 4px 8px; border-radius: 8px; color: var(--wait); background: var(--wait-soft); border: 1px solid var(--wait-line); }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; text-decoration: underline; cursor: pointer; }
  .link:disabled { opacity: 0.45; cursor: not-allowed; }
  .act { padding: 4px 12px; border-radius: 8px; border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--txt); font: inherit; cursor: pointer; }
  .act:disabled { opacity: 0.45; cursor: not-allowed; }
</style>
