<script lang="ts">
  import { untrack } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import { ssh } from '../../lib/stores/ssh.svelte'
  import type { Server } from '../../lib/types'

  /** Modulo di un server: nuovo (`server` assente), salvato o da `~/.ssh/config` (solo password). */
  let { server: serverProp = null, onclose }: { server?: Server | null; onclose: () => void } = $props()
  // il modulo parte dai valori del server al momento dell'apertura (il padre lo ricrea con `{#key}`)
  const server = untrack(() => serverProp)

  const readOnly = server?.source === 'config'
  let name = $state(server?.name ?? '')
  let host = $state(server?.host ?? '')
  let user = $state(server?.user ?? '')
  let port = $state(server?.port ? String(server.port) : '')
  let identityFile = $state(server?.identityFile ?? '')
  let startDir = $state(server?.startDir ?? '')
  let password = $state('')
  let error = $state<string | null>(null)
  let busy = $state(false)
  // `hasPassword` dall'elenco aggiornato (dopo salvataggio/rimozione)
  const hasPassword = $derived(server ? (ssh.servers.find((s) => s.id === server.id)?.hasPassword ?? server.hasPassword) : false)

  async function submit(ev: Event) {
    ev.preventDefault()
    error = null
    busy = true
    try {
      let id = server?.id
      if (!readOnly) {
        const portNum = port.trim() ? Number(port.trim()) : null
        if (portNum !== null && !Number.isInteger(portNum)) throw 'Porta non valida (1–65535)'
        const saved = await ssh.save({
          id: server?.id ?? null,
          name: name.trim(),
          host: host.trim(),
          user: user.trim() || null,
          port: portNum,
          identityFile: identityFile.trim() || null,
          startDir: startDir.trim() || null,
        })
        id = saved.id
      }
      if (password && id) {
        const pw = password
        password = ''
        await ssh.setPassword(id, pw)
      }
      onclose()
    } catch (e) {
      error = errorText(e)
    } finally {
      password = ''
      busy = false
    }
  }

  async function clearPw() {
    if (!server) return
    error = null
    try {
      await ssh.clearPassword(server.id)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<form class="form" onsubmit={submit}>
  {#if readOnly}<p class="hint">Definito in ~/.ssh/config: si può solo gestire la password.</p>{/if}
  <label>Nome<input bind:value={name} disabled={readOnly} placeholder="Server di prova" autocomplete="off" /></label>
  <label>Host<input bind:value={host} disabled={readOnly} placeholder="example.test" autocomplete="off" /></label>
  <label>Utente<input bind:value={user} disabled={readOnly} placeholder="dev" autocomplete="off" /></label>
  <label>Porta<input bind:value={port} disabled={readOnly} inputmode="numeric" placeholder="22" autocomplete="off" /></label>
  <label>Chiave (percorso)<input bind:value={identityFile} disabled={readOnly} placeholder="~/.ssh/id_ed25519" autocomplete="off" /></label>
  <label>Cartella iniziale<input bind:value={startDir} disabled={readOnly} placeholder="/home/dev" autocomplete="off" /></label>
  <label>
    Password (facoltativa)
    <input type="password" bind:value={password} autocomplete="new-password" placeholder={hasPassword ? '••••••••' : ''} />
  </label>
  {#if hasPassword}
    <p class="hint">
      Password salvata nel Portachiavi ·
      <button type="button" class="link" use:tooltip={'Rimuove la password dal Portachiavi'} onclick={() => void clearPw()}>Rimuovi</button>
    </p>
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <div class="actions">
    <button type="button" use:tooltip={'Chiude il modulo senza salvare'} onclick={onclose}>Annulla</button>
    <button type="submit" class="primary" disabled={busy} use:tooltip={'Salva il server'}>Salva</button>
  </div>
</form>

<style>
  .form { display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; font-size: 12px; }
  label { display: flex; flex-direction: column; gap: 3px; color: var(--dim); }
  input {
    font: inherit; font-size: 12.5px; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft);
    border-radius: 6px; padding: 5px 8px; outline: none;
  }
  input:focus { border-color: var(--accent); }
  input:disabled { opacity: 0.6; }
  .hint { margin: 0; color: var(--faint); }
  .err { margin: 0; color: var(--err); overflow-wrap: anywhere; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  button { font: inherit; font-size: 12px; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft); border-radius: 6px; padding: 4px 12px; cursor: pointer; }
  button.primary { background: var(--accent-soft); border-color: var(--accent-line); }
  button.link { background: none; border: 0; padding: 0; color: var(--accent); }
  button:disabled { opacity: 0.5; cursor: default; }
</style>
