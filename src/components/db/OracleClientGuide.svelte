<script lang="ts">
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import { openExternal } from '../../lib/external'
  import * as ipc from '../../lib/ipc'
  import { db } from '../../lib/stores/db.svelte'

  /** Guida a Oracle Instant Client: scheda comprimibile nel pannello o finestra dopo una connessione fallita.
   *  Lo stato vive nello store (`db.oracleClient`); a caricamento riuscito lo store riprova la connessione in attesa. */
  let { modal = false, onclose }: { modal?: boolean; onclose?: () => void } = $props()

  export const DOWNLOAD_URL = 'https://www.oracle.com/database/technologies/instant-client/macos-arm64-downloads.html'

  const status = $derived(db.oracleClient)
  let open = $state(true)
  let busy = $state(false)
  let local = $state<string | null>(null)
  let copied = $state(false)

  const err = $derived(status?.error ?? null)
  /** Comando di rimedio per la quarantena: è il testo fra apici inversi del messaggio del core. */
  const quarantineCmd = $derived(err && /quarantena/i.test(err) ? (err.match(/`([^`]+)`/)?.[1] ?? null) : null)
  const needsRestart = $derived(!!err && /riavvia/i.test(err))
  const detected = $derived(status?.detected ?? [])

  async function run(f: () => Promise<unknown>) {
    if (busy) return
    busy = true
    local = null
    try {
      await f()
    } catch (e) {
      local = errorText(e)
    } finally {
      busy = false
    }
  }
  // anche una cartella trovata passa dal selettore di macOS: la libreria si carica solo da una scelta confermata
  const pick = (start?: string) =>
    run(async () => {
      const st = await ipc.dbOracleClientPick(start)
      if (st) await db.applyClientStatus(st)
    })

  async function copy(cmd: string) {
    try {
      await navigator.clipboard.writeText(cmd)
      copied = true
      setTimeout(() => (copied = false), 1500)
    } catch {
      copied = false
    }
  }
</script>

{#snippet body()}
  <ol class="steps">
    <li>
      Scarica «Instant Client for macOS (ARM64) — Basic Light» dalla pagina di Oracle:
      <button class="link" use:tooltip={'Apre la pagina di download di Oracle nel browser'} onclick={() => openExternal(DOWNLOAD_URL)}>pagina di download</button>
    </li>
    <li>Apri il file <code>.dmg</code> ed esegui <code>install_ic.sh</code>, oppure copia la cartella dove vuoi.</li>
    <li>
      Scegli la cartella di Instant Client:
      <button disabled={busy} use:tooltip={'Apre il selettore di cartelle di macOS'} onclick={() => void pick()}>Scegli la cartella…</button>
    </li>
  </ol>
  {#if detected.length}
    <div class="det">
      <span class="dim">Cartelle trovate (si apre il selettore su quella cartella, conferma con «Apri»):</span>
      {#each detected as d (d)}
        <button class="path" disabled={busy} use:tooltip={'Apre il selettore su questa cartella'} onclick={() => void pick(d)}>{d}</button>
      {/each}
    </div>
  {/if}
  <p class="state" role="status">
    {#if status?.loaded}
      <span class="ok">Caricato</span>{#if status.dir} · <code>{status.dir}</code>{/if}
    {:else if err}
      <span class="bad">Errore</span> · {err}
    {:else if status?.dir}
      <span>Trovato</span> · <code>{status.dir}</code>
    {:else}
      <span class="dim">Nessuna cartella scelta</span>
    {/if}
  </p>
  {#if quarantineCmd}
    <div class="cmd">
      <code>{quarantineCmd}</code>
      <button use:tooltip={'Copia il comando negli appunti'} onclick={() => void copy(quarantineCmd)}>{copied ? 'Copiato' : 'Copia'}</button>
    </div>
  {/if}
  {#if needsRestart}<p class="note">Cambiando cartella dopo il primo caricamento serve riavviare l'app.</p>{/if}
  {#if local}<p class="bad" role="alert">{local}</p>{/if}
{/snippet}

{#if modal}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" data-no-tooltip="sfondo: clic per chiudere" onclick={() => onclose?.()}>
    <div class="modal" role="dialog" aria-modal="true" aria-label="Oracle Instant Client" tabindex="-1" data-no-tooltip="finestra di dialogo" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === 'Escape' && onclose?.()}>
      <h3>Serve Oracle Instant Client</h3>
      {@render body()}
      <div class="foot"><button use:tooltip={'Chiude la guida'} onclick={() => onclose?.()}>Chiudi</button></div>
    </div>
  </div>
{:else}
  <section class="card" aria-label="Oracle Instant Client">
    <button class="head" aria-expanded={open} use:tooltip={'Mostra o nasconde la guida'} onclick={() => (open = !open)}>Oracle Instant Client: non caricato</button>
    {#if open}<div class="in">{@render body()}</div>{/if}
  </section>
{/if}

<style>
  .card { margin: 4px 12px 8px; border: 1px solid var(--edge-soft); border-radius: 8px; background: var(--wait-soft); font-size: 12px; }
  .head { width: 100%; text-align: left; background: none; border: 0; color: var(--wait); font: inherit; font-weight: 600; padding: 6px 10px; cursor: pointer; }
  .in { padding: 0 10px 8px; }
  .steps { margin: 0 0 6px; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; }
  .det { display: flex; flex-direction: column; align-items: flex-start; gap: 3px; margin-bottom: 4px; }
  .path { font: 11.5px ui-monospace, Menlo, monospace; max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .state { margin: 4px 0; overflow-wrap: anywhere; }
  .ok { color: var(--work); font-weight: 600; }
  .bad { color: var(--err); font-weight: 600; overflow-wrap: anywhere; }
  .dim { color: var(--dim); }
  .note { margin: 4px 0; color: var(--wait); }
  .cmd { display: flex; align-items: center; gap: 6px; margin: 4px 0; }
  .cmd code { flex: 1; min-width: 0; overflow-wrap: anywhere; padding: 3px 6px; background: var(--glass); border-radius: 6px; }
  code { font: 11.5px ui-monospace, Menlo, monospace; }
  button { font: inherit; font-size: 11.5px; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft); border-radius: 6px; padding: 2px 8px; cursor: pointer; }
  button:disabled { opacity: 0.5; cursor: default; }
  button.link { background: none; border: 0; padding: 0; color: var(--accent); text-decoration: underline; }
  button.head { background: none; border: 0; }
  .scrim { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, 0.45); display: flex; align-items: center; justify-content: center; }
  .modal { width: min(560px, 92vw); max-height: 80vh; overflow: auto; background: var(--panel, #1e1e22); border: 1px solid var(--edge-soft); border-radius: 10px; padding: 12px 14px; font-size: 12.5px; }
  h3 { margin: 0 0 8px; font-size: 13px; }
  .foot { display: flex; justify-content: flex-end; padding-top: 8px; }
</style>
