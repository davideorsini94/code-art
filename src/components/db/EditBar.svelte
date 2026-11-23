<script lang="ts">
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import * as ipc from '../../lib/ipc'
  import { fmtInt } from '../../lib/db/grid'
  import type { PgRowEdit } from '../../lib/types'

  let {
    connId,
    schema,
    table,
    edits,
    count,
    oracle = false,
    onapplied,
    onclear,
  }: { connId: string; schema: string; table: string; edits: PgRowEdit[]; count: number; oracle?: boolean; onapplied: () => void; onclear: () => void } = $props()

  let busy = $state(false)
  let error = $state<string | null>(null)
  let info = $state<string | null>(null)
  let preview = $state<string[] | null>(null)

  async function showPreview() {
    if (busy) return
    busy = true
    error = null
    info = null
    try {
      preview = await (oracle ? ipc.dbOraPreviewEdits : ipc.dbPgPreviewEdits)(connId, schema, table, edits)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  async function apply() {
    if (busy) return
    busy = true
    error = null
    info = null
    preview = null
    try {
      await (oracle ? ipc.dbOraApplyEdits : ipc.dbPgApplyEdits)(connId, schema, table, edits)
      onapplied()
    } catch (e) {
      const msg = errorText(e)
      // "annullato": conferma rifiutata nella finestra nativa, le modifiche restano
      if (/annullat/i.test(msg)) info = 'Applicazione annullata: le modifiche restano in sospeso'
      else error = msg
    } finally {
      busy = false
    }
  }
</script>

<div class="eb" role="region" aria-label="Modifiche in sospeso">
  <span class="cnt">{fmtInt(count)} {count === 1 ? 'modifica' : 'modifiche'}</span>
  <button disabled={busy} use:tooltip={'Mostra le istruzioni SQL che verrebbero eseguite, senza eseguirle'} onclick={() => void showPreview()}>Anteprima</button>
  <button class="apply" disabled={busy} use:tooltip={'Applica le modifiche in una transazione (chiede conferma)'} onclick={() => void apply()}>Applica</button>
  <button disabled={busy} use:tooltip={'Scarta tutte le modifiche in sospeso'} onclick={onclear}>Annulla tutto</button>
  {#if oracle}<span class="dim">In Oracle una stringa vuota è NULL</span>{/if}
  {#if error}<span class="err" role="alert">{error}</span>{/if}
  {#if info}<span class="dim" role="status">{info}</span>{/if}
</div>

{#if preview}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" data-no-tooltip="sfondo: clic per chiudere" onclick={() => (preview = null)}>
    <div class="modal" role="dialog" aria-modal="true" aria-label="Anteprima delle istruzioni" tabindex="-1" data-no-tooltip="finestra di dialogo" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === 'Escape' && (preview = null)}>
      <h3>Anteprima delle istruzioni</h3>
      <ul class="sql">
        {#each preview as s}<li>{s}</li>{/each}
      </ul>
      <div class="foot"><button use:tooltip={'Chiude l\'anteprima'} onclick={() => (preview = null)}>Chiudi</button></div>
    </div>
  </div>
{/if}

<style>
  .eb { display: flex; align-items: center; gap: 8px; padding: 5px 10px; flex: none; border-bottom: 1px solid var(--edge-soft); background: color-mix(in srgb, #fbbf24 10%, transparent); font-size: 12px; }
  .eb button, .foot button { font-size: 12px; padding: 2px 9px; border-radius: 6px; border: 1px solid var(--edge-soft); background: transparent; color: inherit; cursor: pointer; }
  .eb button:disabled { opacity: 0.5; cursor: default; }
  .apply { color: var(--work, #4ade80); }
  .cnt { font-weight: 600; }
  .err { color: var(--err, #f87171); white-space: pre-wrap; }
  .dim { color: var(--dim); }
  .scrim { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, 0.45); display: flex; align-items: center; justify-content: center; }
  .modal { width: min(720px, 90vw); max-height: 70vh; display: flex; flex-direction: column; background: var(--panel, #1e1e22); border: 1px solid var(--edge-soft); border-radius: 10px; padding: 12px 14px; }
  h3 { margin: 0 0 8px; font-size: 13px; }
  .sql { margin: 0; padding: 0; list-style: none; overflow: auto; font: 12px ui-monospace, Menlo, monospace; }
  .sql li { padding: 4px 0; border-bottom: 1px solid var(--edge-soft); white-space: pre-wrap; word-break: break-all; }
  .foot { display: flex; justify-content: flex-end; padding-top: 8px; }
</style>
