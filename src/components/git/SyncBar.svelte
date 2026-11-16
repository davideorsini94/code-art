<script lang="ts">
  import { tooltip } from '../../lib/tooltip'
  import { git, REMOTE_LABEL } from '../../lib/stores/git.svelte'
  import type { GitRemoteOp, GitRepoStatus } from '../../lib/types'

  /** `compact`: branch e ahead/behind stanno già nell'intestazione della sezione. */
  let { repo, status, compact = false }: { repo: string; status: GitRepoStatus; compact?: boolean } = $props()

  const mine = $derived(git.busy?.repo === repo ? git.busy : null)
  const remoteBusy = $derived(mine !== null && mine.op in REMOTE_LABEL)
  const ops: { op: GitRemoteOp; label: string; tip: string }[] = [
    { op: 'fetch', label: 'Fetch', tip: 'Scarica le novità del server senza toccare i tuoi file' },
    { op: 'pull', label: 'Pull', tip: 'Scarica e integra le novità (solo avanzamento semplice, mai unioni a sorpresa)' },
    { op: 'push', label: 'Push', tip: 'Invia i tuoi commit al server (mai con la forza)' },
  ]
</script>

<div class="sync">
  {#if !compact}
  <button class="branch" use:tooltip={'Cambia branch'} onclick={() => git.openPicker('switch', repo)}>
    ⎇ {status.branch ?? (status.detached ? 'HEAD staccato' : 'nessun branch')}
  </button>
  {#if status.ahead > 0 || status.behind > 0}
    <span class="ab" use:tooltip={`${status.ahead} commit da inviare, ${status.behind} da scaricare`}>↑{status.ahead} ↓{status.behind}</span>
  {/if}
  {/if}
  <span class="spacer"></span>
  {#each ops as o (o.op)}
    <button class="op" disabled={git.busy !== null} use:tooltip={o.tip} onclick={() => void git.remote(repo, o.op)}>{o.label}</button>
  {/each}
</div>
{#if remoteBusy && mine}
  <div class="progress" role="status">
    <span class="line">{REMOTE_LABEL[mine.op as GitRemoteOp]}{mine.line ? ` · ${mine.line}` : '…'}</span>
    <button class="cancel" use:tooltip={'Interrompe l’operazione'} onclick={() => void git.cancel()}>Annulla</button>
  </div>
{/if}

<style>
  .sync { display: flex; align-items: center; gap: 4px; }
  .spacer { flex: 1; }
  .branch { max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 3px 8px; border-radius: 8px; border: 1px solid var(--edge-soft); background: var(--glass); color: var(--txt); font: inherit; font-size: 12px; font-family: ui-monospace, Menlo, monospace; cursor: pointer; }
  .branch:hover { background: var(--glass2); }
  .ab { font-size: 11.5px; color: var(--dim); font-variant-numeric: tabular-nums; }
  .op { padding: 3px 7px; border-radius: 6px; border: 1px solid transparent; background: none; color: var(--dim); font: inherit; font-size: 11.5px; cursor: pointer; }
  .op:hover:not(:disabled) { background: var(--glass); color: var(--txt); }
  .op:disabled { opacity: 0.45; cursor: not-allowed; }
  .progress { display: flex; align-items: center; gap: 8px; font-size: 11.5px; color: var(--dim); }
  .line { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cancel { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; text-decoration: underline; cursor: pointer; }
  @media (prefers-reduced-motion: no-preference) { .branch, .op { transition: background var(--dur); } }
</style>
