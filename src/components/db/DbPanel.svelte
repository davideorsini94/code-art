<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from '../Icon.svelte'
  import InfoToast from '../InfoToast.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import { db } from '../../lib/stores/db.svelte'
  import { ssh } from '../../lib/stores/ssh.svelte'
  import DbForm from './DbForm.svelte'
  import DbTree from './DbTree.svelte'
  import type { DbConnection } from '../../lib/types'

  let form = $state<{ connection: DbConnection | null } | null>(null)
  let promptPw = $state('')
  let openIds = $state<Record<string, boolean>>({})

  onMount(() => {
    void db.init()
    void ssh.init() // elenco dei server per il tunnel
  })

  const STATE_LABEL = { connecting: 'Connessione…', connected: 'Connesso', disconnected: 'Non connesso', error: 'Errore' } as const

  async function sendPassword(ev: Event) {
    ev.preventDefault()
    const id = db.passwordFor
    const pw = promptPw
    promptPw = ''
    if (!id || !pw) return
    try {
      await db.setPassword(id, pw, true)
    } catch (e) {
      db.errors[id] = errorText(e)
    }
  }

  async function removeConnection(c: DbConnection) {
    try {
      await db.remove(c.id)
    } catch (e) {
      db.errors[c.id] = errorText(e)
    }
  }
</script>

<div class="panel">
  <div class="head">
    <span class="title">Database</span>
    <button class="add" aria-label="Aggiungi connessione" use:tooltip={'Aggiungi una connessione'} onclick={() => (form = { connection: null })}><Icon name="plus" size={14} /> connessione</button>
  </div>

  {#if form}
    {#key form.connection?.id ?? 'new'}
      <DbForm connection={form.connection} onclose={() => (form = null)} />
    {/key}
  {/if}

  {#if db.passwordFor}
    {@const target = db.connections.find((c) => c.id === db.passwordFor)}
    <form class="pw" onsubmit={sendPassword}>
      <span>Password per {target?.name ?? 'la connessione'}</span>
      <input type="password" aria-label="Password" bind:value={promptPw} autocomplete="new-password" />
      <div class="row2">
        <button type="button" use:tooltip={'Annulla la connessione'} onclick={() => { promptPw = ''; db.passwordFor = null }}>Annulla</button>
        <button type="submit" class="primary" use:tooltip={'Salva la password nel Portachiavi e connetti'}>Connetti</button>
      </div>
    </form>
  {/if}

  {#if db.loaded && db.connections.length === 0}
    <p class="empty">Nessuna connessione. Aggiungine una con «+ connessione».</p>
  {/if}

  <ul class="list">
    {#each db.connections as c (c.id)}
      {@const st = db.stateOf(c.id)}
      {@const msg = db.errors[c.id] ?? db.states[c.id]?.message}
      <li>
        <div class="srv">
          <button class="name" aria-expanded={!!openIds[c.id]} use:tooltip={`${c.kind === 'postgres' ? 'Postgres' : 'MongoDB'} · ${c.user ? c.user + '@' : ''}${c.host}:${c.port}`} onclick={() => (openIds[c.id] = !openIds[c.id])}>
            <span class="dot {st}" use:tooltip={msg ? `${STATE_LABEL[st]}: ${msg}` : STATE_LABEL[st]} role="img" aria-label={STATE_LABEL[st]}></span>
            <span class="label">{c.name}</span>
            {#if c.acceptInvalidCerts}<span class="tag warn" use:tooltip={'Accetta certificati non validi'}>cert.</span>{/if}
          </button>
          {#if st === 'connected'}
            <button class="act" use:tooltip={'Chiude la connessione'} onclick={() => void db.disconnect(c.id)}>Disconnetti</button>
          {:else if st === 'connecting'}
            <button class="act" disabled use:tooltip={'Connessione in corso'}>Connessione…</button>
          {:else}
            <button class="act" use:tooltip={st === 'error' ? 'Riprova la connessione' : 'Connetti al database'} onclick={() => void db.connect(c.id)}>{st === 'error' ? 'Riconnetti' : 'Connetti'}</button>
          {/if}
          <button class="act" aria-label="Modifica {c.name}" use:tooltip={'Modifica la connessione'} onclick={() => (form = { connection: c })}>Modifica</button>
          <button class="act" aria-label="Elimina {c.name}" use:tooltip={'Rimuove la connessione dall\'elenco'} onclick={() => void removeConnection(c)}><Icon name="x" size={12} /></button>
        </div>
        {#if msg && st === 'error'}<p class="err">{msg}</p>{/if}
        {#if openIds[c.id]}
          {#if st !== 'connected' && st !== 'connecting'}
            <div class="bar">
              <span>Connessione chiusa</span>
              <button class="act" use:tooltip={'Riconnetti al database'} onclick={() => void db.connect(c.id)}>Riconnetti</button>
            </div>
          {/if}
          {#if db.trees[c.id] || st === 'connected'}
            <DbTree connectionId={c.id} kind={c.kind} disabled={st !== 'connected'} />
          {/if}
        {/if}
      </li>
    {/each}
  </ul>
</div>
<InfoToast message={db.notice} onclose={() => (db.notice = null)} />

<style>
  .panel { height: 100%; overflow: auto; font-size: 12.5px; }
  .head { display: flex; align-items: center; justify-content: space-between; padding: 8px 12px; }
  .title { font-weight: 650; }
  .list { list-style: none; margin: 0; padding: 0 4px; }
  .empty { margin: 0; padding: 16px; color: var(--faint); text-align: center; }
  .srv { display: flex; align-items: center; gap: 4px; padding: 2px 4px; }
  .name { flex: 1; min-width: 0; display: flex; align-items: center; gap: 7px; background: none; border: 0; color: var(--txt); font: inherit; text-align: left; cursor: pointer; padding: 4px 4px; border-radius: 6px; }
  .name:hover { background: var(--glass); }
  .label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tag { font-size: 10.5px; color: var(--faint); border: 1px solid var(--edge-soft); border-radius: 999px; padding: 0 6px; white-space: nowrap; }
  .tag.warn { color: var(--wait); border-color: var(--wait); background: var(--wait-soft); }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; background: var(--faint); opacity: 0.5; }
  .dot.connected { background: var(--work); opacity: 1; box-shadow: 0 0 8px var(--work); }
  .dot.connecting { background: var(--wait); opacity: 1; animation: pulse 1.2s ease-in-out infinite; }
  .dot.error { background: var(--err); opacity: 1; }
  @keyframes pulse { 50% { opacity: 0.3; } }
  @media (prefers-reduced-motion: reduce) { .dot.connecting { animation: none; } }
  button { font: inherit; }
  .act, .add, .pw button { font-size: 11.5px; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft); border-radius: 6px; padding: 2px 8px; cursor: pointer; display: inline-flex; align-items: center; gap: 3px; }
  .act:disabled { opacity: 0.5; cursor: default; }
  .primary { background: var(--accent-soft) !important; border-color: var(--accent-line) !important; }
  .err { margin: 0 12px 4px; color: var(--err); font-size: 12px; overflow-wrap: anywhere; }
  .bar { display: flex; align-items: center; justify-content: space-between; margin: 2px 8px; padding: 4px 8px; border-radius: 6px; background: var(--wait-soft); color: var(--wait); font-size: 12px; }
  .pw { display: flex; flex-direction: column; gap: 6px; margin: 4px 12px 8px; padding: 8px; border: 1px solid var(--edge-soft); border-radius: 8px; }
  .pw input { font: inherit; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft); border-radius: 6px; padding: 5px 8px; outline: none; }
  .row2 { display: flex; justify-content: flex-end; gap: 8px; }
</style>
