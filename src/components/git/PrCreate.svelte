<script lang="ts">
  import { onMount } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { links } from '../../lib/conv/links'
  import { renderMarkdown } from '../../lib/safe-markdown'
  import { github } from '../../lib/stores/github.svelte'
  import { gitLog } from '../../lib/ipc'

  let { repo }: { repo: string } = $props()

  let title = $state('')
  let body = $state('')
  let base = $state('')
  let draft = $state(false)
  let preview = $state(false)

  const reason = $derived(title.trim() === '' ? 'Scrivi un titolo' : github.busy ? 'Un’operazione GitHub è in corso' : '')

  // titolo dall'ultimo commit (se il campo è ancora vuoto quando arriva)
  onMount(() => {
    void gitLog(repo, 1)
      .then((list) => {
        if (title === '' && list[0]) title = list[0].subject
      })
      .catch(() => {})
  })

  async function create() {
    if (reason) return
    await github.createPr(repo, title.trim(), body, base.trim(), draft)
  }
</script>

<div class="pr-create" aria-label="Nuova pull request" role="group">
  <input class="field" bind:value={title} aria-label="Titolo della pull request" placeholder="Titolo" />
  <input class="field" bind:value={base} aria-label="Branch di destinazione" placeholder="Base: branch predefinito del repository" />
  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={!preview} class:on={!preview} use:tooltip={'Scrivi la descrizione in markdown'} onclick={() => (preview = false)}>Scrivi</button>
    <button role="tab" aria-selected={preview} class:on={preview} use:tooltip={'Mostra la descrizione come apparirà'} onclick={() => (preview = true)}>Anteprima</button>
  </div>
  {#if preview}
    <div class="md preview" role="region" aria-label="Anteprima della descrizione" use:links>{@html renderMarkdown(body)}</div>
  {:else}
    <textarea class="field" bind:value={body} rows="5" aria-label="Descrizione (markdown)" placeholder="Descrizione (markdown, facoltativa)"></textarea>
  {/if}
  <div class="row">
    <label use:tooltip={'Crea la pull request come bozza: non è ancora pronta per la revisione'}><input type="checkbox" bind:checked={draft} aria-label="Bozza" /> Bozza</label>
    <span class="grow"></span>
    <button class="link" use:tooltip={'Chiudi senza creare la pull request'} onclick={() => (github.creating = null)}>Annulla</button>
    <button class="act" disabled={reason !== ''} use:tooltip={reason || 'Invia il branch se serve e crea la pull request su GitHub'} onclick={() => void create()}>
      {github.busy === 'create' ? 'Creazione…' : 'Crea pull request'}
    </button>
  </div>
</div>

<style>
  .pr-create { display: flex; flex-direction: column; gap: 6px; padding: 8px; border-radius: 8px; background: var(--surface-1); border: 1px solid var(--edge-soft); }
  .field { padding: 5px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); color: var(--txt); font: inherit; font-size: 12px; outline: 0; }
  .field:focus { border-color: var(--accent); }
  .field::placeholder { color: var(--faint); }
  textarea.field { resize: vertical; min-height: 70px; }
  .tabs { display: flex; gap: 4px; }
  .tabs button { padding: 2px 10px; border-radius: 6px; border: 1px solid transparent; background: none; color: var(--dim); font: inherit; font-size: 11.5px; cursor: pointer; }
  .tabs button.on { color: var(--txt); border-color: var(--edge-soft); background: var(--glass); }
  .preview { min-height: 70px; max-height: 240px; overflow: auto; padding: 6px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); }
  .row { display: flex; align-items: center; gap: 8px; }
  .grow { flex: 1; }
  label { display: flex; align-items: center; gap: 5px; font-size: 11.5px; color: var(--dim); cursor: pointer; }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; font-size: 12px; text-decoration: underline; cursor: pointer; }
  .act { padding: 4px 12px; border-radius: 8px; border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--txt); font: inherit; font-size: 12px; cursor: pointer; }
  .act:disabled { opacity: 0.45; cursor: not-allowed; }
</style>
