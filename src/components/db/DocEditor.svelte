<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import * as ipc from '../../lib/ipc'

  type ICode = import('monaco-editor').editor.IStandaloneCodeEditor

  let {
    connId,
    database,
    collection,
    doc,
    onclose,
    ondone,
  }: {
    connId: string
    database: string
    collection: string
    /** JSON esteso rilassato del documento; `null` = nuovo documento. */
    doc: string | null
    onclose: () => void
    ondone: () => void
  } = $props()

  const isNew = untrack(() => doc === null)
  const initial = untrack(() => doc ?? '{\n  \n}')
  let text = initial
  let host = $state<HTMLDivElement>()
  let ed: ICode | undefined
  let busy = $state(false)
  let error = $state<string | null>(null)
  let info = $state<string | null>(null)

  /** `_id` del documento originale, come testo JSON. */
  function originalId(): string {
    const o = JSON.parse(doc!)
    if (o === null || typeof o !== 'object' || !('_id' in o)) throw new Error('Il documento non ha il campo _id')
    return JSON.stringify(o._id)
  }

  function parsed(): string | null {
    try {
      const v = JSON.parse(text)
      if (v === null || typeof v !== 'object' || Array.isArray(v)) throw new Error('Il documento deve essere un oggetto JSON')
      return text
    } catch (e) {
      error = `JSON non valido: ${e instanceof Error ? e.message : String(e)}`
      return null
    }
  }

  async function act(f: () => Promise<unknown>) {
    if (busy) return
    busy = true
    info = null
    try {
      await f()
      ondone()
    } catch (e) {
      const msg = errorText(e)
      if (/annullat/i.test(msg)) info = 'Operazione annullata'
      else error = msg
    } finally {
      busy = false
    }
  }

  function save() {
    error = null
    const t = parsed()
    if (t === null) return
    if (isNew) return act(() => ipc.dbMongoInsert(connId, database, collection, t))
    let id: string
    try {
      id = originalId()
    } catch (e) {
      error = errorText(e)
      return
    }
    return act(() => ipc.dbMongoReplace(connId, database, collection, id, t))
  }

  function remove() {
    error = null
    let id: string
    try {
      id = originalId()
    } catch (e) {
      error = errorText(e)
      return
    }
    return act(() => ipc.dbMongoDelete(connId, database, collection, id))
  }

  onMount(() => {
    let dead = false
    void import('../../lib/monaco').then(({ monaco }) => {
      if (dead || !host) return
      ed = monaco.editor.create(host, {
        value: initial,
        language: 'json',
        theme: 'codeart',
        automaticLayout: true,
        minimap: { enabled: false },
        fontSize: 13,
        fontFamily: 'ui-monospace, Menlo, monospace',
        scrollBeyondLastLine: false,
      })
      ed.onDidChangeModelContent(() => {
        text = ed!.getValue()
        error = null
      })
    })
    return () => {
      dead = true
      ed?.dispose()
    }
  })
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" data-no-tooltip="sfondo: clic per chiudere" onclick={onclose}>
  <div class="modal" role="dialog" aria-modal="true" aria-label={isNew ? 'Nuovo documento' : 'Modifica documento'} tabindex="-1" data-no-tooltip="finestra di dialogo" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === 'Escape' && onclose()}>
    <h3>{isNew ? 'Nuovo documento' : 'Modifica documento'} <span class="dim">{database}.{collection}</span></h3>
    <div class="ed" bind:this={host}></div>
    {#if error}<div class="err" role="alert">{error}</div>{/if}
    {#if info}<div class="dim" role="status">{info}</div>{/if}
    <div class="foot">
      {#if !isNew}<button class="del" disabled={busy} use:tooltip={'Elimina il documento dalla collezione (chiede conferma)'} onclick={() => void remove()}>Elimina documento</button>{/if}
      <span class="grow"></span>
      <button disabled={busy} use:tooltip={'Chiude senza salvare'} onclick={onclose}>Annulla</button>
      <button class="ok" disabled={busy} use:tooltip={isNew ? 'Inserisce il documento (chiede conferma)' : 'Sostituisce il documento (chiede conferma)'} onclick={() => void save()}>Salva</button>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, 0.45); display: flex; align-items: center; justify-content: center; }
  .modal { width: min(760px, 92vw); height: min(560px, 80vh); display: flex; flex-direction: column; background: var(--panel, #1e1e22); border: 1px solid var(--edge-soft); border-radius: 10px; padding: 12px 14px; gap: 8px; }
  h3 { margin: 0; font-size: 13px; }
  .dim { color: var(--dim); font-weight: 400; font-size: 12px; }
  .ed { flex: 1; min-height: 0; border: 1px solid var(--edge-soft); border-radius: 6px; overflow: hidden; }
  .err { color: var(--err, #f87171); font-size: 12px; white-space: pre-wrap; }
  .foot { display: flex; gap: 8px; align-items: center; }
  .grow { flex: 1; }
  .foot button { font-size: 12px; padding: 3px 10px; border-radius: 6px; border: 1px solid var(--edge-soft); background: transparent; color: inherit; cursor: pointer; }
  .foot button:disabled { opacity: 0.5; cursor: default; }
  .ok { color: var(--work, #4ade80); }
  .del { color: var(--err, #f87171); }
</style>
