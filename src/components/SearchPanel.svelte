<script lang="ts">
  import { onMount, tick } from 'svelte'
  import FileIcon from './FileIcon.svelte'
  import { tooltip } from '../lib/tooltip'
  import { iconFor } from '../lib/fileicons'
  import { relativeToFolders } from '../lib/conv/paths'
  import { search } from '../lib/stores/search.svelte'
  import { editor } from '../lib/stores/editor.svelte'
  import { projectStore } from '../lib/stores/project.svelte'
  import type { SearchHit } from '../lib/types'

  let input = $state<HTMLInputElement>()
  let filters = $state(false)
  const trusted = $derived(projectStore.untrusted.length === 0)
  const folders = $derived(projectStore.project?.folders ?? [])

  onMount(() => {
    const id = projectStore.project?.id
    if (id) void search.bind(id)
    input?.focus()
    return () => search.unbind()
  })

  // ⌘⇧F con la vista già aperta: il focus torna sul campo
  $effect(() => {
    void search.focusTick
    void tick().then(() => { input?.focus(); input?.select() })
  })

  const name = (p: string) => p.split('/').pop() ?? p
  const dir = (p: string) => {
    const rel = relativeToFolders(p, folders)
    const i = rel.lastIndexOf('/')
    return i < 0 ? '' : rel.slice(0, i)
  }
  /** Parti del testo dell'anteprima: sempre testo, mai HTML. */
  function parts(h: SearchHit) {
    const a = h.previewCol
    const b = a + h.previewLen
    return { before: h.preview.slice(0, a).trimStart(), match: h.preview.slice(a, b), after: h.preview.slice(b) }
  }
  const fmt = (n: number) => n.toLocaleString('it-IT').replace(/\./g, ' ')
  const toggles: { key: 'caseSensitive' | 'wholeWord' | 'regex'; label: string; tip: string }[] = [
    { key: 'caseSensitive', label: 'Aa', tip: 'Distingui maiuscole e minuscole' },
    { key: 'wholeWord', label: 'ab', tip: 'Solo parole intere' },
    { key: 'regex', label: '.*', tip: 'Espressione regolare' },
  ]
</script>

<div class="search">
  <div class="field">
    <input bind:this={input} type="text" placeholder="Cerca" aria-label="Cerca nel progetto" value={search.text}
      oninput={(e) => search.setText(e.currentTarget.value)} onkeydown={(e) => e.key === 'Enter' && search.setText(search.text, true)} />
    {#each toggles as t (t.key)}
      <button class="tog" class:on={search[t.key]} aria-pressed={search[t.key]} aria-label={t.tip} use:tooltip={t.tip}
        onclick={() => { search[t.key] = !search[t.key]; search.schedule() }}>{t.label}</button>
    {/each}
  </div>
  {#if trusted}
    <div class="field">
      <input type="text" placeholder="Sostituisci" aria-label="Sostituisci con" bind:value={search.replacement} />
      <button class="tog" aria-label="Sostituisci tutto" disabled={search.groups.length === 0 || search.replacing || search.status === 'running'}
        use:tooltip={'Sostituisci in tutti i file dei risultati (chiede conferma)'} onclick={() => void search.replaceAll()}>Tutto</button>
    </div>
  {/if}
  <button class="more" aria-expanded={filters} use:tooltip={filters ? 'Nascondi i filtri sui file' : 'Mostra i filtri sui file'} onclick={() => (filters = !filters)}>
    {filters ? '▾' : '▸'} File da includere/escludere
  </button>
  {#if filters}
    <input type="text" placeholder="Includi (es. *.ts, src/**)" aria-label="File da includere" bind:value={search.include} oninput={() => search.schedule()} />
    <input type="text" placeholder="Escludi (es. *.test.ts)" aria-label="File da escludere" bind:value={search.exclude} oninput={() => search.schedule()} />
  {/if}

  <div class="status" role="status">
    {#if search.status === 'error'}<span class="err">{search.error}</span>
    {:else if search.status === 'running'}Ricerca in corso…
    {:else if search.status === 'done'}{search.hitCount === 0 ? 'Nessun risultato' : `${fmt(search.hitCount)} risultati in ${fmt(search.fileCount)} file`}{/if}
    {#if search.truncated}<div class="warn">Mostrati i primi 10 000 risultati</div>{/if}
    {#if search.outcome}
      <div>
        {#if search.outcome.cancelled}Sostituzione annullata
        {:else}Sostituite {search.outcome.replacements} occorrenze in {search.outcome.files} file{/if}
        {#if search.outcome.dirtySkipped > 0}· {search.outcome.dirtySkipped} con modifiche non salvate saltati{/if}
        {#if search.outcome.skipped.length > 0}· {search.outcome.skipped.length} non modificati{/if}
      </div>
    {/if}
  </div>

  <ul class="groups">
    {#each search.groups as g (g.path)}
      {@const spec = iconFor(name(g.path), false, false)}
      <li>
        <button class="gh" aria-expanded={!search.collapsed[g.path]} use:tooltip={relativeToFolders(g.path, folders)} onclick={() => search.toggleGroup(g.path)}>
          <span class="chev">{search.collapsed[g.path] ? '▸' : '▾'}</span>
          <FileIcon id={spec.id} dim={spec.dim} size={14} />
          <span class="gname">{name(g.path)}</span>
          <span class="gdir">{dir(g.path)}</span>
          <span class="count">{g.hits.length}</span>
        </button>
        {#if !search.collapsed[g.path]}
          <ul class="hits">
            {#each g.hits as h, i (i)}
              {@const p = parts(h)}
              <li>
                <button class="hit" use:tooltip={`Riga ${h.line}`} onclick={() => void editor.openAt(h.path, h.line, h.col)}>
                  <span class="ln">{h.line}</span><span class="pv">{p.before}<mark>{p.match}</mark>{p.after}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {/each}
  </ul>
</div>

<style>
  .search { display: flex; flex-direction: column; gap: 6px; padding: 8px; min-height: 0; height: 100%; font-size: 12px; color: var(--txt); }
  .field { display: flex; align-items: center; gap: 4px; padding: 3px 4px 3px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); }
  .field:focus-within, .search > input:focus { border-color: var(--accent); }
  input { flex: 1; min-width: 0; background: none; border: 0; outline: 0; color: var(--txt); font: inherit; }
  .search > input { padding: 5px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); }
  input::placeholder { color: var(--faint); }
  .tog { padding: 2px 6px; border-radius: 6px; border: 1px solid transparent; background: none; color: var(--dim); font: inherit; font-size: 11.5px; cursor: pointer; }
  .tog:hover:not(:disabled) { background: var(--glass); }
  .tog.on { background: var(--accent-soft); border-color: var(--accent-line); color: var(--txt); }
  .tog:disabled { opacity: 0.45; cursor: not-allowed; }
  .more { align-self: flex-start; padding: 0; border: 0; background: none; color: var(--dim); font: inherit; font-size: 11.5px; cursor: pointer; }
  .status { color: var(--dim); font-size: 11.5px; min-height: 14px; }
  .err { color: var(--err); }
  .warn { color: var(--wait); }
  .groups, .hits { list-style: none; margin: 0; padding: 0; }
  .groups { flex: 1; min-height: 0; overflow-y: auto; }
  .gh, .hit { display: flex; align-items: center; gap: 6px; width: 100%; text-align: left; background: none; border: 0; color: inherit; font: inherit; cursor: pointer; border-radius: 6px; }
  .gh { padding: 3px 4px; }
  .hit { padding: 2px 4px 2px 22px; }
  .gh:hover, .hit:hover { background: var(--glass); }
  .chev { width: 10px; color: var(--faint); font-size: 10px; }
  .gname { white-space: nowrap; }
  .gdir { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--faint); font-size: 11px; }
  .count { padding: 0 6px; border-radius: 999px; background: var(--glass2); color: var(--dim); font-size: 10.5px; font-variant-numeric: tabular-nums; }
  .ln { min-width: 24px; color: var(--faint); text-align: right; font-variant-numeric: tabular-nums; }
  .pv { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: pre; font-family: ui-monospace, Menlo, monospace; font-size: 11.5px; color: var(--dim); }
  mark { background: var(--wait-strong); color: var(--txt); border-radius: 2px; }
  @media (prefers-reduced-motion: no-preference) { .field, .tog { transition: border-color var(--dur), background var(--dur); } }
</style>
