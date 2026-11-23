<script lang="ts">
  import { untrack } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import { db } from '../../lib/stores/db.svelte'
  import { ssh } from '../../lib/stores/ssh.svelte'
  import type { DbConnection, DbKind, DbTls } from '../../lib/types'

  /** Modulo di una connessione: nuova (`connection` assente) o salvata. Il padre lo ricrea con `{#key}`. */
  let { connection: connProp = null, onclose }: { connection?: DbConnection | null; onclose: () => void } = $props()
  const connection = untrack(() => connProp)

  const DEFAULT_PORT: Record<DbKind, string> = { postgres: '5432', mongo: '27017' }

  let kind = $state<DbKind>(connection?.kind ?? 'postgres')
  let name = $state(connection?.name ?? '')
  let host = $state(connection?.host ?? '')
  let port = $state(connection ? String(connection.port) : DEFAULT_PORT.postgres)
  let user = $state(connection?.user ?? '')
  let database = $state(connection?.database ?? '')
  let tls = $state<DbTls>(connection?.tls ?? 'prefer')
  let acceptInvalidCerts = $state(connection?.acceptInvalidCerts ?? false)
  let sshServerId = $state(connection?.sshServerId ?? '')
  let password = $state('')
  let error = $state<string | null>(null)
  let busy = $state(false)
  const hasPassword = $derived(connection ? (db.connections.find((c) => c.id === connection.id)?.hasPassword ?? connection.hasPassword) : false)

  const tlsHelp = $derived(
    tls === 'prefer'
      ? kind === 'mongo'
        ? 'TLS per host remoti, senza TLS in locale o con tunnel'
        : 'TLS se il server lo offre'
      : tls === 'require'
        ? 'Connessione rifiutata se il server non offre TLS'
        : 'Nessuna cifratura: solo per reti fidate',
  )

  /** Cambiando tipo, la porta passa a quella predefinita del nuovo tipo se era vuota o quella dell'altro. */
  function setKind(k: DbKind) {
    const other = DEFAULT_PORT[kind]
    if (!port.trim() || port.trim() === other) port = DEFAULT_PORT[k]
    kind = k
  }

  async function submit(ev: Event) {
    ev.preventDefault()
    error = null
    busy = true
    try {
      const portNum = port.trim() ? Number(port.trim()) : null
      if (portNum !== null && (!Number.isInteger(portNum) || portNum < 1 || portNum > 65535)) throw 'Porta non valida (1–65535)'
      const saved = await db.save({
        id: connection?.id,
        kind,
        name: name.trim(),
        host: host.trim(),
        port: portNum,
        user: user.trim() || null,
        database: database.trim() || null,
        tls,
        acceptInvalidCerts,
        sshServerId: sshServerId || null,
      })
      if (password) {
        const pw = password
        password = ''
        await db.setPassword(saved.id, pw)
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
    if (!connection) return
    error = null
    try {
      await db.clearPassword(connection.id)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<form class="form" onsubmit={submit}>
  <label>
    Tipo
    <select value={kind} onchange={(e) => setKind(e.currentTarget.value as DbKind)} use:tooltip={'Postgres o MongoDB'}>
      <option value="postgres">Postgres</option>
      <option value="mongo">MongoDB</option>
    </select>
  </label>
  <label>Nome<input bind:value={name} placeholder="Database di prova" autocomplete="off" /></label>
  <label>Host<input bind:value={host} placeholder="db.example.test" autocomplete="off" /></label>
  <label>Porta<input bind:value={port} inputmode="numeric" placeholder={DEFAULT_PORT[kind]} autocomplete="off" /></label>
  <label>Utente<input bind:value={user} autocomplete="off" /></label>
  <label>Database<input bind:value={database} autocomplete="off" /></label>
  <label>
    TLS
    <select bind:value={tls} use:tooltip={'Cifratura della connessione'}>
      <option value="prefer">Se disponibile (prefer)</option>
      <option value="require">Obbligatorio (require)</option>
      <option value="disable">Disattivato (disable)</option>
    </select>
    <span class="hint">{tlsHelp}</span>
  </label>
  <label class="check warn" use:tooltip={'Attenzione: la connessione accetta anche un certificato falso o scaduto, quindi non è protetta da intercettazioni'}>
    <input type="checkbox" bind:checked={acceptInvalidCerts} />
    Accetta certificati non validi
  </label>
  <label>
    Tunnel SSH
    <select bind:value={sshServerId} use:tooltip={'Server SSH da cui far passare la connessione'}>
      <option value="">Nessuno (diretta)</option>
      {#each ssh.servers as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
    </select>
  </label>
  <label>
    Password (facoltativa, nel Portachiavi)
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
    <button type="submit" class="primary" disabled={busy} use:tooltip={'Salva la connessione'}>Salva</button>
  </div>
</form>

<style>
  .form { display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; font-size: 12px; }
  label { display: flex; flex-direction: column; gap: 3px; color: var(--dim); }
  input:not([type='checkbox']), select {
    font: inherit; font-size: 12.5px; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft);
    border-radius: 6px; padding: 5px 8px; outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); }
  .check { flex-direction: row; align-items: center; gap: 6px; }
  .warn { color: var(--wait); background: var(--wait-soft); border-radius: 6px; padding: 4px 8px; }
  .hint { margin: 0; color: var(--faint); }
  .err { margin: 0; color: var(--err); overflow-wrap: anywhere; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  button { font: inherit; font-size: 12px; color: var(--txt); background: var(--glass); border: 1px solid var(--edge-soft); border-radius: 6px; padding: 4px 12px; cursor: pointer; }
  button.primary { background: var(--accent-soft); border-color: var(--accent-line); }
  button.link { background: none; border: 0; padding: 0; color: var(--accent); }
  button:disabled { opacity: 0.5; cursor: default; }
</style>
