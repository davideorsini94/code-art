<script lang="ts">
  import { tick } from 'svelte'
  import { tooltip } from '../lib/tooltip'
  import { errorText } from '../lib/errors'
  import { ghClone, ghRepoList } from '../lib/ipc'
  import { github, GH_MISSING } from '../lib/stores/github.svelte'
  import type { RemoteRepo } from '../lib/types'

  const LIMIT = 200

  let account = $state('')
  let repos = $state<RemoteRepo[]>([])
  let filter = $state('')
  let loading = $state(false)
  let cloning = $state<string | null>(null)
  let error = $state<string | null>(null)
  let box = $state<HTMLDivElement>()
  let seq = 0

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase()
    return q ? repos.filter((r) => r.nameWithOwner.toLowerCase().includes(q) || r.description.toLowerCase().includes(q)) : repos
  })

  // all'apertura: account di gh, poi i repository dell'account attivo
  $effect(() => {
    if (!github.cloneOpen) return
    account = ''
    filter = ''
    error = null
    void github.init().then(() => {
      if (!github.missing) void load()
    })
    void tick().then(() => box?.querySelector('input')?.focus())
  })

  async function load() {
    const my = ++seq
    loading = true
    error = null
    try {
      const list = await ghRepoList(account, LIMIT)
      if (my === seq) repos = list
    } catch (e) {
      if (my === seq) error = errorText(e)
    } finally {
      if (my === seq) loading = false
    }
  }

  function changeAccount(login: string) {
    account = login
    repos = []
    void load()
  }

  /** Il core chiede dove clonare e apre il progetto in una nuova finestra. */
  async function clone(nwo: string) {
    if (cloning) return
    cloning = nwo
    error = null
    try {
      await ghClone(nwo, account)
      github.cloneOpen = false
    } catch (e) {
      error = errorText(e)
    } finally {
      cloning = null
    }
  }

  const close = () => (github.cloneOpen = false)
</script>

{#if github.cloneOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" data-no-tooltip="sfondo: un clic fuori dalla finestra la chiude" onclick={close}></div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="dialog"
    data-no-tooltip="finestra di dialogo: Esc la chiude, i controlli interni hanno il loro tooltip"
    role="dialog"
    aria-modal="true"
    aria-label="Clona da GitHub"
    tabindex="-1"
    bind:this={box}
    onkeydown={(e) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        close()
      }
    }}
  >
    <header>
      <h2>Clona da GitHub</h2>
      <button class="x" aria-label="Chiudi" use:tooltip={'Chiudi (Esc)'} onclick={close}>×</button>
    </header>
    {#if github.missing}
      <div class="msg">{GH_MISSING}</div>
    {:else}
      <div class="row">
        <select class="field" aria-label="Account GitHub" value={account} use:tooltip={'Account di gh di cui elencare i repository'} onchange={(e) => changeAccount(e.currentTarget.value)}>
          <option value="">Account attivo di gh{github.activeLogin ? ` (${github.activeLogin})` : ''}</option>
          {#each github.accounts.filter((a) => !a.active) as a (a.login)}<option value={a.login}>{a.login}</option>{/each}
        </select>
        <input class="field grow" bind:value={filter} aria-label="Filtra i repository" placeholder="Filtra…" />
      </div>
      {#if error}<div class="err" role="alert">{error}</div>{/if}
      <ul class="list">
        {#each shown as r (r.nameWithOwner)}
          <li>
            <div class="info">
              <span class="name">{r.nameWithOwner}</span>
              {#if r.private}<span class="badge">privato</span>{/if}
              {#if r.description}<span class="desc">{r.description}</span>{/if}
            </div>
            <button class="act" aria-label={`Clona ${r.nameWithOwner}`} disabled={cloning !== null} use:tooltip={'Scegli dove clonarlo; si apre in una nuova finestra'} onclick={() => void clone(r.nameWithOwner)}>
              {cloning === r.nameWithOwner ? 'Clonazione…' : 'Clona'}
            </button>
          </li>
        {:else}
          <li class="msg">{loading ? 'Caricamento…' : 'Nessun repository'}</li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .scrim { position: fixed; inset: 0; z-index: 60; background: var(--scrim); }
  .dialog { position: fixed; z-index: 61; top: 12vh; left: 50%; transform: translateX(-50%); width: min(560px, calc(100vw - 32px)); max-height: 70vh; display: flex; flex-direction: column; gap: 8px; padding: 14px; border-radius: var(--radius); background: var(--surface-pop); border: 1px solid var(--edge); box-shadow: var(--panel-shadow); color: var(--txt); font-size: 12px; outline: 0; }
  header { display: flex; align-items: center; justify-content: space-between; }
  h2 { margin: 0; font-size: 14px; font-weight: 650; }
  .x { padding: 0 4px; border: 0; background: none; color: var(--dim); font: inherit; font-size: 16px; cursor: pointer; }
  .x:hover { color: var(--txt); }
  .row { display: flex; gap: 6px; }
  .grow { flex: 1; }
  .field { min-width: 0; padding: 5px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); color: var(--txt); font: inherit; outline: 0; }
  .field:focus { border-color: var(--accent); }
  .field::placeholder { color: var(--faint); }
  .list { list-style: none; margin: 0; padding: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 4px; }
  .list li { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 8px; background: var(--surface-1); }
  .info { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px; }
  .name { font-weight: 600; overflow-wrap: anywhere; }
  .badge { font-size: 10.5px; color: var(--wait); }
  .desc { flex-basis: 100%; color: var(--dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .msg { color: var(--dim); }
  .err { color: var(--err); padding: 4px 8px; border-radius: 8px; background: var(--err-soft); white-space: pre-wrap; }
  .act { flex: none; padding: 4px 12px; border-radius: 8px; border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--txt); font: inherit; cursor: pointer; }
  .act:disabled { opacity: 0.45; cursor: not-allowed; }
  @media (prefers-reduced-motion: no-preference) { .dialog { animation: dlg-in var(--dur) ease-out; } }
  @keyframes dlg-in { from { opacity: 0; transform: translate(-50%, 6px); } }
</style>
