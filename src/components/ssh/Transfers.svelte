<script lang="ts">
  import Icon from '../Icon.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { ssh } from '../../lib/stores/ssh.svelte'
  import type { TransferState } from '../../lib/types'

  let open = $state<Record<string, boolean>>({})

  const LABEL: Record<TransferState, string> = { running: 'In corso', done: 'Completato', cancelled: 'Annullato', error: 'Non riuscito' }
  const pct = (done: number, total: number) => (total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0)
  const nameOf = (p: string) => p.slice(p.lastIndexOf('/') + 1) || p
</script>

{#if ssh.transfers.length > 0}
  <div class="transfers" aria-label="Trasferimenti">
    {#each ssh.transfers as t (t.transferId)}
      {@const errs = t.errors ?? []}
      <div class="tr" class:bad={t.state === 'error' || errs.length > 0}>
        <div class="line">
          <span class="file" use:tooltip={t.file}>{t.file ? nameOf(t.file) : 'Trasferimento'}</span>
          <span class="st">{LABEL[t.state]}{t.state === 'running' ? ` ${pct(t.done, t.total)}%` : ''}</span>
          {#if t.state === 'running'}
            <button class="act" use:tooltip={'Interrompe il trasferimento (i file già copiati restano)'} onclick={() => void ssh.cancelTransfer(t.transferId)}>Annulla</button>
          {:else if errs.length > 0 || t.state === 'error'}
            <button class="act" aria-label="Chiudi" use:tooltip={'Chiudi'} onclick={() => ssh.dismiss(t.transferId)}><Icon name="x" size={12} /></button>
          {/if}
        </div>
        {#if t.state === 'running'}
          <div class="bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct(t.done, t.total)}><span style:width="{pct(t.done, t.total)}%"></span></div>
        {/if}
        {#if t.message}<p class="msg">{t.message}</p>{/if}
        {#if errs.length > 0}
          <button class="more" use:tooltip={'Mostra o nascondi gli errori per file'} aria-expanded={!!open[t.transferId]} onclick={() => (open[t.transferId] = !open[t.transferId])}>
            {errs.length === 1 ? '1 errore' : `${errs.length} errori`}
          </button>
          {#if open[t.transferId]}
            <ul class="errs">
              {#each errs as e (e.path)}<li><b>{nameOf(e.path)}</b>: {e.message}</li>{/each}
            </ul>
          {/if}
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .transfers { position: sticky; bottom: 0; display: flex; flex-direction: column; gap: 6px; padding: 8px 12px; border-top: 1px solid var(--edge-soft); background: var(--bg, inherit); font-size: 12px; }
  .line { display: flex; align-items: center; gap: 8px; }
  .file { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .st { color: var(--dim); white-space: nowrap; }
  .bad .st { color: var(--err); }
  .act, .more { font: inherit; font-size: 11.5px; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft); border-radius: 6px; padding: 1px 8px; cursor: pointer; display: inline-flex; align-items: center; }
  .more { align-self: flex-start; background: none; border: 0; color: var(--err); padding: 0; }
  .bar { height: 4px; border-radius: 2px; background: var(--glass2); overflow: hidden; margin-top: 4px; }
  .bar span { display: block; height: 100%; background: var(--accent); transition: width 150ms linear; }
  @media (prefers-reduced-motion: reduce) { .bar span { transition: none; } }
  .msg, .errs { margin: 2px 0 0; color: var(--err); overflow-wrap: anywhere; }
  .errs { padding-left: 16px; }
</style>
