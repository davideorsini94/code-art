<script lang="ts">
  import { onMount } from 'svelte'
  import PrCreate from './PrCreate.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { checkColor, checksText, github, GH_MISSING, MERGE_LABEL, STATE_LABEL } from '../../lib/stores/github.svelte'
  import type { MergeMethod, PrState } from '../../lib/types'

  let { repo }: { repo: string } = $props()

  const remote = $derived(github.remotes[repo])
  const prs = $derived(github.prs[repo] ?? [])
  const folderName = $derived(repo.replace(/\/+$/, '').split('/').pop() ?? '')

  // "Pubblica su GitHub"
  let publishing = $state(false)
  let pubName = $state('')
  let pubPrivate = $state(true)
  let pubDescription = $state('')

  // "Unisci" su una pull request
  let merging = $state<number | null>(null)
  let method = $state<MergeMethod>('merge')
  let deleteBranch = $state(false)
  let openChecks = $state<number | null>(null)

  onMount(() => {
    void github.init().then(() => github.refreshRepo(repo))
  })

  function startPublish() {
    pubName = folderName
    pubPrivate = true
    pubDescription = ''
    publishing = true
  }

  async function publish() {
    if (await github.publish(repo, pubName.trim(), pubPrivate, pubDescription.trim())) publishing = false
  }

  function startMerge(n: number) {
    merging = n
    method = 'merge'
    deleteBranch = false
  }

  async function merge(n: number) {
    if (await github.merge(repo, n, method, deleteBranch)) merging = null
  }

  function toggleChecks(n: number) {
    openChecks = openChecks === n ? null : n
    if (openChecks !== null) void github.loadChecks(repo, n)
  }
</script>

<section class="gh" aria-label="GitHub">
  <header>
    <h4>GitHub</h4>
    {#if remote}<span class="nwo">{remote}</span>{/if}
  </header>
  {#if github.missing}
    <div class="msg">{GH_MISSING}</div>
  {:else if remote === null}
    {#if !publishing}
      <button class="act" use:tooltip={'Crea un repository su GitHub da questo, lo collega come «origin» e invia i commit'} onclick={startPublish}>Pubblica su GitHub</button>
    {:else}
      <div class="form">
        <input class="field" bind:value={pubName} aria-label="Nome del repository" placeholder="Nome del repository" />
        <input class="field" bind:value={pubDescription} aria-label="Descrizione del repository" placeholder="Descrizione (facoltativa)" />
        <div class="row">
          <label use:tooltip={'Solo tu (e chi inviti) potrete vedere il repository'}><input type="checkbox" bind:checked={pubPrivate} aria-label="Privato" /> Privato</label>
          <span class="grow"></span>
          <button class="link" use:tooltip={'Chiudi senza pubblicare'} onclick={() => (publishing = false)}>Annulla</button>
          <button class="act" disabled={pubName.trim() === '' || github.busy !== null} use:tooltip={'Crea il repository su GitHub e invia i commit'} onclick={() => void publish()}>
            {github.busy === 'publish' ? 'Pubblicazione…' : 'Pubblica'}
          </button>
        </div>
      </div>
    {/if}
  {:else if remote}
    <div class="row">
      <select class="field" aria-label="Account GitHub" value={github.account} use:tooltip={'Account di gh usato per questo progetto'} onchange={(e) => void github.setAccount(e.currentTarget.value)}>
        <option value="">Account attivo di gh{github.activeLogin ? ` (${github.activeLogin})` : ''}</option>
        {#each github.accounts as a (a.login)}<option value={a.login}>{a.login}</option>{/each}
      </select>
    </div>
    <div class="row">
      <select class="field" aria-label="Pull request da mostrare" value={github.prState} use:tooltip={'Quali pull request mostrare'} onchange={(e) => void github.setPrState(e.currentTarget.value as PrState)}>
        {#each Object.entries(STATE_LABEL) as [v, l] (v)}<option value={v}>{l}</option>{/each}
      </select>
      <button class="icon" aria-label="Aggiorna le pull request" disabled={github.loading} use:tooltip={'Aggiorna le pull request'} onclick={() => void github.refreshPrs(repo)}>↻</button>
      <span class="grow"></span>
      <button class="act" disabled={github.creating === repo} use:tooltip={'Apre una pull request dal branch corrente'} onclick={() => (github.creating = repo)}>Nuova pull request</button>
    </div>
    {#if github.creating === repo}<PrCreate {repo} />{/if}
    {#if prs.length === 0}
      <div class="msg">{github.loading ? 'Caricamento…' : 'Nessuna pull request'}</div>
    {:else}
      <ul class="prs">
        {#each prs as pr (pr.number)}
          <li>
            <div class="pr">
              <button class="dot {checkColor(pr.checks)}" aria-label={`Controlli della pull request #${pr.number}`} use:tooltip={checksText(pr.checks)} onclick={() => toggleChecks(pr.number)}></button>
              <button class="title" use:tooltip={`Apri #${pr.number} nel browser`} onclick={() => github.open(pr.url)}>{pr.title}</button>
              {#if pr.state === 'open'}
                <button class="link" use:tooltip={'Unisci questa pull request'} onclick={() => startMerge(pr.number)}>Unisci…</button>
              {/if}
            </div>
            <div class="meta">
              #{pr.number} · {pr.head} → {pr.base} · {pr.author}{pr.isDraft ? ' · bozza' : ''}{pr.state !== 'open' ? ` · ${pr.state === 'merged' ? 'unita' : 'chiusa'}` : ''}
            </div>
            {#if openChecks === pr.number}
              <ul class="checks">
                {#each github.checks[`${repo}#${pr.number}`] ?? [] as c (c.name + c.url)}
                  <li>
                    <span class="dot small {c.state === 'passing' ? 'ok' : c.state === 'failing' ? 'fail' : c.state === 'pending' ? 'wait' : 'none'}"></span>
                    <button class="title" use:tooltip={'Apri il controllo nel browser'} onclick={() => github.open(c.url)}>{c.name}</button>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if merging === pr.number}
              <div class="row merge">
                <select class="field" aria-label="Metodo di unione" bind:value={method} use:tooltip={'Come unire i commit nel branch di destinazione'}>
                  {#each Object.entries(MERGE_LABEL) as [v, l] (v)}<option value={v}>{l}</option>{/each}
                </select>
                <label use:tooltip={'Elimina il branch della pull request dopo l’unione'}><input type="checkbox" bind:checked={deleteBranch} aria-label="Elimina il branch dopo l’unione" /> Elimina branch</label>
                <span class="grow"></span>
                <button class="link" use:tooltip={'Chiudi senza unire'} onclick={() => (merging = null)}>Annulla</button>
                <button class="act" aria-label={`Unisci #${pr.number}`} disabled={github.busy !== null} use:tooltip={'Unisci (verrà chiesta conferma)'} onclick={() => void merge(pr.number)}>Unisci</button>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

<style>
  .gh { display: flex; flex-direction: column; gap: 6px; padding-top: 6px; border-top: 1px solid var(--edge-soft); }
  header { display: flex; align-items: baseline; gap: 8px; }
  h4 { margin: 0; font-size: 11px; font-weight: 650; text-transform: uppercase; letter-spacing: 0.04em; color: var(--dim); }
  .nwo { font-size: 11.5px; color: var(--faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .msg { color: var(--dim); }
  .form { display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .grow { flex: 1; }
  .field { min-width: 0; padding: 4px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); color: var(--txt); font: inherit; font-size: 12px; outline: 0; }
  .field:focus { border-color: var(--accent); }
  .field::placeholder { color: var(--faint); }
  label { display: flex; align-items: center; gap: 5px; font-size: 11.5px; color: var(--dim); cursor: pointer; }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; font-size: 12px; text-decoration: underline; cursor: pointer; white-space: nowrap; }
  .act { align-self: flex-start; padding: 4px 12px; border-radius: 8px; border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--txt); font: inherit; font-size: 12px; cursor: pointer; white-space: nowrap; }
  .act:disabled, .icon:disabled { opacity: 0.45; cursor: not-allowed; }
  .icon { padding: 2px 6px; border-radius: 6px; border: 1px solid var(--edge-soft); background: none; color: var(--dim); font: inherit; cursor: pointer; }
  .prs, .checks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .checks { gap: 2px; padding-left: 16px; }
  .checks li { display: flex; align-items: center; gap: 6px; }
  .pr { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .title { flex: 1; min-width: 0; padding: 0; border: 0; background: none; color: var(--txt); font: inherit; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; }
  .title:hover { color: var(--accent); }
  .meta { padding-left: 16px; font-size: 11px; color: var(--faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .merge { padding-left: 16px; }
  .dot { flex: none; width: 10px; height: 10px; padding: 0; border-radius: 50%; border: 0; background: var(--faint); cursor: pointer; }
  .dot.small { width: 7px; height: 7px; cursor: default; }
  .dot.ok { background: var(--work); }
  .dot.fail { background: var(--err); }
  .dot.wait { background: var(--wait); }
  .dot.none { background: var(--edge); }
  @media (prefers-reduced-motion: no-preference) { .dot { transition: background-color var(--dur) ease; } }
</style>
