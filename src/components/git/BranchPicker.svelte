<script lang="ts">
  import { onMount } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { gitBranches } from '../../lib/ipc'
  import { errorText } from '../../lib/errors'
  import { branchNameError, git, type PickerMode } from '../../lib/stores/git.svelte'
  import type { GitBranch } from '../../lib/types'

  let { repo, mode = 'switch', onclose }: { repo: string; mode?: PickerMode; onclose: () => void } = $props()

  let branches = $state<GitBranch[]>([])
  let loadError = $state<string | null>(null)
  let filter = $state('')
  let merging = $state(false)
  let creating = $state(false)
  let newName = $state('')
  let input = $state<HTMLInputElement>()

  const current = $derived(branches.find((b) => b.current && !b.remote)?.name ?? null)
  const shown = $derived(branches.filter((b) => !b.name.endsWith('/HEAD') && b.name.toLowerCase().includes(filter.trim().toLowerCase())))
  const nameError = $derived(branchNameError(newName))
  const exists = $derived(branches.some((b) => !b.remote && b.name === newName))
  const createError = $derived(newName === '' ? null : (nameError ?? (exists ? 'Esiste già un branch con questo nome' : null)))
  const canCreate = $derived(newName !== '' && !createError && git.busy === null)

  onMount(() => {
    git.error = null // niente errori vecchi: quelli mostrati qui sono dell'operazione che si sta per fare
    git.notice = null
    merging = mode === 'merge'
    creating = mode === 'create'
    gitBranches(repo).then((b) => (branches = b), (e) => (loadError = errorText(e)))
    input?.focus()
  })

  async function pick(b: GitBranch) {
    if (merging) {
      if (b.name === current) return
      const out = await git.merge(repo, b.name)
      // si chiude solo se l'unione è riuscita: con conflitti o errore il messaggio resta qui
      if (out && out.kind !== 'conflicts') onclose()
    } else if (!b.current && (await git.checkout(repo, b.name))) onclose()
  }

  async function create() {
    if (!canCreate) return
    if (await git.createBranch(repo, newName, current)) onclose()
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="picker" data-no-tooltip="contenitore del selettore: Esc lo chiude" role="dialog" aria-label="Branch" tabindex="-1" onkeydown={(e) => e.key === 'Escape' && onclose()}>
  <input bind:this={input} type="text" placeholder={merging ? 'Unisci in quello corrente: scegli un branch' : 'Cerca un branch'} aria-label="Filtra i branch" bind:value={filter} />
  {#if loadError}<div class="err">{loadError}</div>{/if}
  {#if git.error}<div class="err" role="alert">{git.error}</div>{/if}
  {#if git.notice}<div class="note" role="status">{git.notice}</div>{/if}
  <ul>
    {#each shown as b (`${b.remote}:${b.name}`)}
      <li>
        <button class="item" class:cur={b.current} disabled={merging && b.name === current} use:tooltip={b.current ? 'Branch corrente' : merging ? `Unisci ${b.name} in ${current ?? 'quello corrente'}` : b.remote ? 'Crea un branch locale che segue questo' : 'Passa a questo branch'} onclick={() => void pick(b)}>
          <span class="mark">{b.current ? '✓' : ''}</span>
          <span class="bname">{b.name}</span>
          {#if b.remote}<span class="tag">remoto</span>{/if}
        </button>
      </li>
    {/each}
    {#if shown.length === 0 && !loadError}<li class="empty">Nessun branch</li>{/if}
  </ul>
  <div class="foot">
    <button class="link" aria-pressed={merging} use:tooltip={'Il prossimo branch scelto verrà unito in quello corrente'} onclick={() => { merging = !merging; creating = false }}>
      {merging ? '✓ ' : ''}Unisci in quello corrente…
    </button>
    {#if !creating}
      <button class="link" use:tooltip={'Crea un nuovo branch a partire da quello corrente'} onclick={() => { creating = true; merging = false }}>Crea branch da qui…</button>
    {:else}
      <div class="create">
        <input type="text" placeholder="Nome del nuovo branch" aria-label="Nome del nuovo branch" aria-invalid={createError !== null} bind:value={newName} onkeydown={(e) => e.key === 'Enter' && void create()} />
        <button class="go" disabled={!canCreate} use:tooltip={createError ?? `Crea il branch e passa a esso${current ? `, partendo da ${current}` : ''}`} onclick={() => void create()}>Crea</button>
      </div>
      {#if createError}<div class="err" role="alert">{createError}</div>{/if}
    {/if}
  </div>
</div>

<style>
  .picker { display: flex; flex-direction: column; gap: 6px; width: 300px; max-height: 380px; padding: 8px; border-radius: var(--radius); background: var(--surface-pop); border: 1px solid var(--edge); box-shadow: var(--panel-shadow); font-size: 12px; color: var(--txt); }
  input { padding: 5px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); color: var(--txt); font: inherit; outline: 0; min-width: 0; }
  input:focus { border-color: var(--accent); }
  input[aria-invalid='true'] { border-color: var(--err); }
  input::placeholder { color: var(--faint); }
  ul { list-style: none; margin: 0; padding: 0; overflow-y: auto; min-height: 0; }
  .item { display: flex; align-items: center; gap: 6px; width: 100%; padding: 3px 4px; border: 0; border-radius: 6px; background: none; color: inherit; font: inherit; cursor: pointer; text-align: left; }
  .item:hover:not(:disabled) { background: var(--glass); }
  .item:disabled { opacity: 0.5; cursor: default; }
  .cur { color: var(--accent); }
  .mark { width: 12px; }
  .bname { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: ui-monospace, Menlo, monospace; }
  .tag { padding: 0 6px; border-radius: 999px; background: var(--glass2); color: var(--dim); font-size: 10.5px; }
  .empty { padding: 4px; color: var(--faint); }
  .foot { display: flex; flex-direction: column; gap: 4px; border-top: 1px solid var(--edge-soft); padding-top: 6px; }
  .link { align-self: flex-start; padding: 0; border: 0; background: none; color: var(--dim); font: inherit; cursor: pointer; }
  .link:hover { color: var(--txt); }
  .create { display: flex; gap: 4px; }
  .create input { flex: 1; }
  .go { padding: 4px 12px; border-radius: 8px; border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--txt); font: inherit; cursor: pointer; }
  .go:disabled { opacity: 0.45; cursor: not-allowed; }
  .err { color: var(--err); font-size: 11.5px; white-space: pre-wrap; }
  .note { color: var(--wait); font-size: 11.5px; }
</style>
