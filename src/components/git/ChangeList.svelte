<script lang="ts">
  import { untrack } from 'svelte'
  import FileIcon from '../FileIcon.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { iconFor } from '../../lib/fileicons'
  import { git } from '../../lib/stores/git.svelte'
  import { editor } from '../../lib/stores/editor.svelte'
  import { fsRead } from '../../lib/ipc'
  import { parseConflicts } from '../../lib/conflicts'
  import type { GitStatusEntry } from '../../lib/types'

  let { repo, title, entries, staged }: { repo: string; title: string; entries: GitStatusEntry[]; staged: boolean } = $props()

  const name = (p: string) => p.split('/').pop() ?? p
  const dir = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '')
  const letter = (e: GitStatusEntry) => (staged ? (e.staged ?? '?') : e.conflict ? '!' : e.untracked ? 'U' : (e.unstaged ?? '?'))
  const LABEL: Record<string, string> = { M: 'Modificato', A: 'Aggiunto', D: 'Eliminato', R: 'Rinominato', C: 'Copiato', T: 'Tipo cambiato', U: 'Non tracciato', '!': 'In conflitto' }
  const paths = $derived(entries.map((e) => e.path))
  const blocked = $derived(git.busy !== null)
  let open = $state(true)

  // conflitti pronti da segnare come risolti: letti dal disco, senza marcatori e senza modifiche non salvate nell'editor.
  // Un file si rilegge quando cambia lo stato (a ogni aggiornamento dello stato git) o la sua scheda, non quando cambiano le altre.
  let ready = $state<Record<string, boolean>>({})
  const token: Record<string, number> = {}
  const seen: Record<string, string> = {}
  function check(p: string) {
    const abs = git.absolute(repo, p)
    const n = (token[p] = (token[p] ?? 0) + 1)
    if (editor.tabs.some((t) => t.key === abs && t.dirty)) {
      ready[p] = false
      return
    }
    Promise.resolve()
      .then(() => fsRead(abs))
      .then((c) => { if (token[p] === n) ready[p] = !c.text.includes('<<<<<<<') || parseConflicts(c.text).length === 0 })
      .catch(() => { if (token[p] === n) ready[p] = false })
  }
  $effect(() => {
    const todo = entries.filter((e) => e.conflict).map((e) => e.path)
    untrack(() => todo.forEach(check))
  })
  $effect(() => {
    const todo = entries.filter((e) => e.conflict).map((e) => e.path)
    const sigs = todo.map((p) => {
      const t = editor.tabs.find((x) => x.key === git.absolute(repo, p))
      return [p, `${t?.dirty ?? '-'}:${t?.mtimeMs ?? '-'}`] as const
    })
    untrack(() => {
      for (const [p, sig] of sigs) {
        if (seen[p] !== undefined && seen[p] !== sig) check(p)
        seen[p] = sig
      }
    })
  })
</script>

<section class="group" aria-label={title}>
  <div class="head">
    <button class="toggle" aria-expanded={open} use:tooltip={open ? 'Comprimi l’elenco' : 'Espandi l’elenco'} onclick={() => (open = !open)}>
      <span class="chev">{open ? '▾' : '▸'}</span>{title}<span class="count">{entries.length}</span>
    </button>
    <span class="acts">
      {#if staged}
        <button aria-label="Togli tutto dallo stage" disabled={blocked || entries.length === 0} use:tooltip={'Togli tutto dallo stage'} onclick={() => void git.unstage(repo, paths)}>−</button>
      {:else}
        <button aria-label="Scarta tutte le modifiche" disabled={blocked || entries.length === 0} use:tooltip={'Scarta tutte le modifiche (chiede conferma)'} onclick={() => void git.discard(repo, paths)}>↶</button>
        <button aria-label="Aggiungi tutto allo stage" disabled={blocked || entries.length === 0} use:tooltip={'Aggiungi tutto allo stage'} onclick={() => void git.stage(repo, paths)}>＋</button>
      {/if}
    </span>
  </div>
  {#if open}
    <ul>
      {#each entries as e (e.path)}
        {@const spec = iconFor(name(e.path), false, false)}
        {@const l = letter(e)}
        <li class="row">
          <button class="main" use:tooltip={e.origPath ? `${e.origPath} → ${e.path}` : e.path} onclick={() => void git.openDiff(repo, e, staged)}>
            <FileIcon id={spec.id} dim={spec.dim} size={14} />
            <span class="fname">{name(e.path)}</span>
            <span class="fdir">{dir(e.path)}</span>
          </button>
          {#if !staged && e.conflict && ready[e.path]}
            <button class="solve" aria-label="Segna {name(e.path)} come risolto" disabled={blocked} use:tooltip={'Nessun conflitto rimasto: aggiungi il file allo stage'} onclick={() => void git.stage(repo, [e.path])}>Segna come risolto</button>
          {/if}
          <span class="acts hover">
            <button aria-label="Apri il file {name(e.path)}" use:tooltip={'Apri il file'} onclick={() => void editor.open(git.absolute(repo, e.path))}>↗</button>
            {#if !staged}
              <button aria-label="Scarta le modifiche a {name(e.path)}" disabled={blocked} use:tooltip={'Scarta le modifiche (chiede conferma)'} onclick={() => void git.discard(repo, [e.path])}>↶</button>
              <button aria-label="Aggiungi {name(e.path)} allo stage" disabled={blocked} use:tooltip={'Aggiungi allo stage'} onclick={() => void git.stage(repo, [e.path])}>＋</button>
            {:else}
              <button aria-label="Togli {name(e.path)} dallo stage" disabled={blocked} use:tooltip={'Togli dallo stage'} onclick={() => void git.unstage(repo, [e.path])}>−</button>
            {/if}
          </span>
          <span class="badge s-{l === '!' ? 'conflict' : l}" use:tooltip={LABEL[l] ?? 'Cambiato'}>{l}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .group { font-size: 12px; }
  .head { display: flex; align-items: center; justify-content: space-between; }
  .toggle { display: flex; align-items: center; gap: 5px; flex: 1; padding: 3px 2px; border: 0; background: none; color: var(--dim); font: inherit; font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; cursor: pointer; text-align: left; }
  .chev { width: 10px; font-size: 10px; color: var(--faint); }
  .count { padding: 0 6px; border-radius: 999px; background: var(--glass2); color: var(--dim); font-size: 10.5px; font-weight: 400; letter-spacing: 0; }
  ul { list-style: none; margin: 0; padding: 0; }
  .row { display: flex; align-items: center; gap: 2px; border-radius: 6px; }
  .row:hover { background: var(--glass); }
  .main { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; padding: 2px 4px; border: 0; background: none; color: var(--txt); font: inherit; cursor: pointer; text-align: left; }
  .fname { white-space: nowrap; }
  .fdir { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--faint); font-size: 11px; }
  .acts { display: flex; gap: 1px; }
  .acts button { width: 20px; height: 20px; padding: 0; border: 0; border-radius: 5px; background: none; color: var(--dim); font: inherit; cursor: pointer; }
  .acts button:hover:not(:disabled) { background: var(--glass2); color: var(--txt); }
  .acts button:disabled { opacity: 0.4; cursor: not-allowed; }
  .solve { height: 20px; padding: 0 8px; border: 0; border-radius: 5px; font: inherit; font-size: 11px; background: var(--accent-soft); color: var(--accent); cursor: pointer; }
  .solve:disabled { opacity: 0.4; cursor: not-allowed; }
  .hover { visibility: hidden; }
  .row:hover .hover, .row:focus-within .hover { visibility: visible; }
  .badge { width: 16px; text-align: center; font-size: 11px; font-weight: 600; font-family: ui-monospace, Menlo, monospace; color: var(--wait); }
  .s-A, .s-U { color: var(--work); }
  .s-D, .s-conflict { color: var(--err); }
  .s-R { color: var(--accent); }
</style>
