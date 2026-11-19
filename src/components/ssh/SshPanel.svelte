<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from '../Icon.svelte'
  import InfoToast from '../InfoToast.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import { asFsError, sshHome } from '../../lib/ipc'
  import { ssh, resolveRoot } from '../../lib/stores/ssh.svelte'
  import ServerForm from './ServerForm.svelte'
  import RemoteTree from './RemoteTree.svelte'
  import Transfers from './Transfers.svelte'
  import type { Server } from '../../lib/types'

  let form = $state<{ server: Server | null } | null>(null)
  let promptPw = $state('')
  const roots = $derived(ssh.roots)
  const resolving = new Set<string>()
  let openIds = $state<Record<string, boolean>>({})

  onMount(() => void ssh.init())

  const STATE_LABEL = { connecting: 'Connessione…', connected: 'Connesso', disconnected: 'Non connesso', error: 'Errore' } as const

  // radice dell'albero: cartella iniziale, altrimenti la home del server; sempre assoluta (`~` risolta con la home),
  // così i percorsi dell'albero si confrontano con la radice (trascinamenti compresi)
  $effect(() => {
    for (const s of ssh.servers) {
      if (ssh.stateOf(s.id) !== 'connected' || roots[s.id] || resolving.has(s.id)) continue
      const sd = s.startDir
      if (sd && sd.trim().startsWith('/')) {
        ssh.roots[s.id] = resolveRoot(sd, '/')
        continue
      }
      resolving.add(s.id)
      sshHome(s.id)
        .then((h) => {
          // server modificato nel frattempo: si rifà al prossimo giro
          if (ssh.servers.find((x) => x.id === s.id)?.startDir === sd && ssh.stateOf(s.id) === 'connected') ssh.roots[s.id] = resolveRoot(sd, h)
        })
        .catch((e) => (ssh.errors[s.id] = errorText(asFsError(e).message)))
        .finally(() => resolving.delete(s.id))
    }
  })

  async function sendPassword(ev: Event) {
    ev.preventDefault()
    const id = ssh.passwordFor
    const pw = promptPw
    promptPw = ''
    if (!id || !pw) return
    try {
      await ssh.setPassword(id, pw, true)
    } catch (e) {
      ssh.errors[id] = errorText(e)
    }
  }

  async function removeServer(s: Server) {
    try {
      await ssh.remove(s.id)
    } catch (e) {
      ssh.errors[s.id] = errorText(e)
    }
  }
</script>

<div class="panel">
  <div class="head">
    <span class="title">Server SSH</span>
    <button class="add" aria-label="Aggiungi server" use:tooltip={'Aggiungi un server'} onclick={() => (form = { server: null })}><Icon name="plus" size={14} /> server</button>
  </div>

  {#if form}
    {#key form.server?.id ?? 'new'}
      <ServerForm server={form.server} onclose={() => (form = null)} />
    {/key}
  {/if}

  {#if ssh.passwordFor}
    {@const target = ssh.servers.find((s) => s.id === ssh.passwordFor)}
    <form class="pw" onsubmit={sendPassword}>
      <span>Password per {target?.name ?? 'il server'}</span>
      <input type="password" aria-label="Password" bind:value={promptPw} autocomplete="new-password" />
      <div class="row2">
        <button type="button" use:tooltip={'Annulla la connessione'} onclick={() => { promptPw = ''; ssh.passwordFor = null }}>Annulla</button>
        <button type="submit" class="primary" use:tooltip={'Salva la password nel Portachiavi e connetti'}>Connetti</button>
      </div>
    </form>
  {/if}

  {#if ssh.loaded && ssh.servers.length === 0}
    <p class="empty">Nessun server. Aggiungine uno con «+ server».</p>
  {/if}

  <ul class="list">
    {#each ssh.servers as s (s.id)}
      {@const st = ssh.stateOf(s.id)}
      {@const msg = ssh.errors[s.id] ?? ssh.states[s.id]?.message}
      <li>
        <div class="srv">
          <button
            class="name"
            aria-expanded={!!openIds[s.id]}
            use:tooltip={`${s.user ? s.user + '@' : ''}${s.host}${s.port ? ':' + s.port : ''}`}
            onclick={() => (openIds[s.id] = !openIds[s.id])}
          >
            <span class="dot {st}" use:tooltip={msg ? `${STATE_LABEL[st]}: ${msg}` : STATE_LABEL[st]} role="img" aria-label={STATE_LABEL[st]}></span>
            <span class="label">{s.name}</span>
            {#if s.source === 'config'}<span class="tag">da ssh config</span>{/if}
          </button>
          {#if st === 'connected'}
            <button class="act" use:tooltip={'Chiude la connessione'} onclick={() => void ssh.disconnect(s.id)}>Disconnetti</button>
          {:else if st === 'connecting'}
            <button class="act" disabled use:tooltip={'Connessione in corso'}>Connessione…</button>
          {:else}
            <button class="act" use:tooltip={st === 'error' ? 'Riprova la connessione' : 'Connetti al server'} onclick={() => void ssh.connect(s.id)}>{st === 'error' ? 'Riconnetti' : 'Connetti'}</button>
          {/if}
          <button class="act" aria-label="Modifica {s.name}" use:tooltip={s.source === 'config' ? 'Password del server' : 'Modifica il server'} onclick={() => (form = { server: s })}>Modifica</button>
          {#if s.source === 'saved'}
            <button class="act" aria-label="Elimina {s.name}" use:tooltip={'Rimuove il server dall\'elenco'} onclick={() => void removeServer(s)}><Icon name="x" size={12} /></button>
          {/if}
        </div>
        {#if msg && st === 'error'}<p class="err">{msg}</p>{/if}
        {#if roots[s.id] && (st === 'connected' || ssh.states[s.id])}
          {#if st !== 'connected'}
            <div class="bar">
              <span>Connessione chiusa</span>
              <button class="act" use:tooltip={'Riconnetti al server'} onclick={() => void ssh.connect(s.id)}>Riconnetti</button>
            </div>
          {/if}
          <RemoteTree serverId={s.id} root={roots[s.id]} disabled={st !== 'connected'} />
        {/if}
      </li>
    {/each}
  </ul>
  <Transfers />
</div>
<InfoToast message={ssh.notice} onclose={() => (ssh.notice = null)} />

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
