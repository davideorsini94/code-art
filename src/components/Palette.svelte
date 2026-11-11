<script lang="ts">
  import { onMount, tick, untrack } from 'svelte'
  import { convSearch, fsAllFiles, settingsGet, settingsSet, type ConvSearchHit } from '../lib/ipc'
  import { palette } from '../lib/palette/state.svelte'
  import {
    absolutePath,
    commands,
    createLatest,
    fuzzyPositions,
    hooks,
    PERMISSION_MODES,
    rankIndices,
    segments,
    substringPositions,
  } from '../lib/palette/search'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { editor } from '../lib/stores/editor.svelte'
  import { projectStore } from '../lib/stores/project.svelte'
  import { tooltip } from '../lib/tooltip'
  import { errorText } from '../lib/errors'

  type Item = { key: string; section: string; label: string; labelPos: number[]; detail?: string; detailPos?: number[]; badge?: string; run: () => void | Promise<void> }

  const MODES_ID = 'settings.permissionMode'
  const TEXT_DELAY = 200

  let query = $state('')
  /** La query usata per ordinare: aggiornata al massimo una volta per frame (50 000 percorsi). */
  let rq = $state('')
  let error = $state<string | null>(null)
  let raf = 0
  let sel = $state(0)
  let files = $state<string[]>([])
  let filesError = $state<string | null>(null)
  let hits = $state<{ q: string; list: ConvSearchHit[] }>({ q: '', list: [] })
  let currentMode = $state<string | null>(null)
  let input = $state<HTMLInputElement>()
  let listEl = $state<HTMLDivElement>()

  let returnTo: HTMLElement | null = null
  let wasOpen = false
  let filesSeq = 0

  const searcher = createLatest(convSearch, (q, list) => (hits = { q, list }), TEXT_DELAY)

  // apertura / cambio di modalità: stato pulito; chiusura: il focus torna dov'era
  $effect(() => {
    const open = palette.open
    const mode = palette.mode
    untrack(() => {
      if (open) {
        if (!wasOpen) returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null
        query = ''
        rq = ''
        error = null
        sel = 0
        hits = { q: '', list: [] }
        searcher.cancel()
        if (mode === 'files' || mode === 'all') void loadFiles()
        if (mode === 'modes') void loadCurrentMode()
        void tick().then(() => input?.focus())
      } else if (wasOpen) {
        searcher.cancel()
        const back = returnTo
        returnTo = null
        const a = document.activeElement
        if (back?.isConnected && (!a || a === document.body || input?.contains(a) || a === input)) back.focus()
      }
      wasOpen = open
    })
  })

  async function loadFiles() {
    const mine = ++filesSeq
    filesError = null
    try {
      const list = await fsAllFiles()
      if (mine === filesSeq) files = list
    } catch {
      if (mine === filesSeq) {
        files = []
        filesError = 'Impossibile elencare i file del progetto'
      }
    }
  }

  async function loadCurrentMode() {
    currentMode = await settingsGet('default_permission_mode').catch(() => null)
  }

  // ricerca nel testo delle conversazioni: solo in ⌘K, dopo 200 ms, solo l'ultima interrogazione
  $effect(() => {
    const q = query.trim()
    const on = palette.open && palette.mode === 'all'
    untrack(() => {
      if (on && Array.from(q).length >= 2) searcher.query(q)
      else searcher.cancel()
    })
  })

  $effect(() => {
    const q = query
    untrack(() => {
      sel = 0
      if (typeof requestAnimationFrame !== 'function') return void (rq = q)
      cancelAnimationFrame(raf)
      raf = requestAnimationFrame(() => (rq = q))
    })
  })

  onMount(() => {
    hooks.openModes = () => {
      palette.mode = 'modes'
    }
    return () => {
      searcher.cancel()
      if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(raf)
    }
  })

  function openFile(rel: string) {
    const folders = projectStore.project?.folders ?? []
    void editor.open(absolutePath(folders, rel))
  }

  function modeItems(q: string): Item[] {
    const idx = rankIndices(q, PERMISSION_MODES.map(([, l]) => l), 10)
    return idx.map((i) => {
      const [mode, label] = PERMISSION_MODES[i]
      return {
        key: `mode:${mode}`,
        section: 'Modalità permessi predefinita',
        label,
        labelPos: fuzzyPositions(q, label),
        badge: mode === currentMode ? 'attuale' : undefined,
        run: () => settingsSet('default_permission_mode', mode),
      }
    })
  }

  function commandItems(q: string, limit: number): Item[] {
    return rankIndices(q, commands.map((c) => c.title), limit).map((i) => ({
      key: `cmd:${commands[i].id}`,
      section: 'Comandi',
      label: commands[i].title,
      labelPos: fuzzyPositions(q, commands[i].title),
      run: () => commands[i].run(),
    }))
  }

  function fileItems(q: string, limit: number): Item[] {
    return rankIndices(q, files, limit).map((i) => {
      const nameStart = files[i].lastIndexOf('/') + 1
      const pos = fuzzyPositions(q, files[i])
      return {
        key: `file:${files[i]}`,
        section: 'File',
        label: files[i].slice(nameStart),
        labelPos: pos.filter((p) => p >= nameStart).map((p) => p - nameStart),
        detail: files[i],
        detailPos: pos,
        run: () => openFile(files[i]),
      }
    })
  }

  function convItems(q: string): Item[] {
    const all = conversations.all
    const mk = (c: (typeof all)[number], extra: Partial<Item>): Item => ({
      key: `conv:${c.sessionId}`,
      section: 'Conversazioni',
      label: c.title,
      labelPos: [],
      badge: c.archived ? 'archiviata' : undefined,
      run: () => void conversations.select(c.sessionId),
      ...extra,
    })
    if (!q) return conversations.list.slice(0, 6).map((c) => mk(c, {}))
    const byTitle = rankIndices(q, all.map((c) => c.title), 8).map((i) => mk(all[i], { labelPos: fuzzyPositions(q, all[i].title) }))
    const seen = new Set(byTitle.map((i) => i.key))
    const byText: Item[] = []
    if (hits.q === q) {
      for (const h of hits.list) {
        const c = all.find((x) => x.sessionId === h.sessionId)
        if (!c || seen.has(`conv:${c.sessionId}`)) continue
        byText.push(mk(c, { detail: h.snippet, detailPos: substringPositions(q, h.snippet) }))
      }
    }
    return [...byTitle, ...byText.slice(0, 8)]
  }

  const items = $derived.by<Item[]>(() => {
    const q = rq.trim()
    switch (palette.mode) {
      case 'files':
        return fileItems(q, 50)
      case 'commands':
        return commandItems(q, 50)
      case 'modes':
        return modeItems(q)
      default:
        return [...convItems(q), ...(q ? fileItems(q, 8) : []), ...commandItems(q, q ? 6 : 20)]
    }
  })

  const placeholder = $derived(
    palette.mode === 'files' ? 'Apri file…' : palette.mode === 'commands' ? 'Esegui un comando…' : palette.mode === 'modes' ? 'Scegli la modalità…' : 'Cerca file, conversazioni, comandi…',
  )

  $effect(() => {
    if (sel >= items.length) sel = Math.max(0, items.length - 1)
  })

  $effect(() => {
    const i = sel
    void items
    void tick().then(() => listEl?.querySelector(`[data-i="${i}"]`)?.scrollIntoView({ block: 'nearest' }))
  })

  async function choose(item: Item | undefined) {
    if (!item) return
    if (item.key === `cmd:${MODES_ID}`) {
      void item.run() // resta aperta: mostra le quattro modalità
      return
    }
    if (item.key.startsWith('mode:')) {
      // la palette resta aperta se la scrittura fallisce, con l'errore visibile
      try {
        await item.run()
        palette.close()
      } catch (e) {
        error = `Impossibile salvare: ${errorText(e)}`
      }
      return
    }
    // prima chiude e riporta il focus, poi esegue: ciò che il comando mette a fuoco vince
    palette.close()
    await tick()
    void item.run()
  }

  function onKey(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229) return
    if (e.key === 'Escape') {
      e.preventDefault()
      palette.close()
    } else if (e.key === 'Tab') {
      e.preventDefault() // il focus resta nel campo
      input?.focus()
    } else if (e.target !== input) {
      return
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      if (items.length) sel = (sel + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length
    } else if (e.key === 'Enter') {
      e.preventDefault()
      void (async () => {
        if (rq !== query) {
          rq = query // Invio prima del prossimo frame: si ordina subito con il testo attuale
          sel = 0
          await tick()
        }
        await choose(items[sel])
      })()
    }
  }
</script>

{#snippet marked(text: string, pos: number[])}
  {#each segments(text, pos) as s, i (i)}{#if s.hit}<mark>{s.t}</mark>{:else}{s.t}{/if}{/each}
{/snippet}

{#if palette.open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="cover" data-no-tooltip="sfondo che chiude la palette: nessun controllo da spiegare" onmousedown={(e) => e.target === e.currentTarget && palette.close()}>
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div class="panel glass-strong" role="dialog" aria-modal="true" aria-label="Ricerca" tabindex="-1" data-no-tooltip="finestra di dialogo: la tastiera è gestita dal campo di ricerca" onkeydown={onKey}>
      <input
        bind:this={input}
        bind:value={query}
        class="q"
        type="text"
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-list"
        aria-activedescendant={items.length ? `palette-opt-${sel}` : undefined}
        aria-label={placeholder}
        {placeholder}
        spellcheck="false"
        autocomplete="off"
      />
      {#if error}<div class="err" role="alert">{error}</div>{/if}
      <div class="list" id="palette-list" role="listbox" bind:this={listEl}>
        {#each items as it, i (it.key)}
          {#if i === 0 || items[i - 1].section !== it.section}
            <div class="sec" role="presentation">{it.section}</div>
          {/if}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            id="palette-opt-{i}"
            data-i={i}
            class="opt"
            class:on={i === sel}
            role="option"
            use:tooltip={it.detail ?? ''}
            tabindex="-1"
            aria-selected={i === sel}
            onmousemove={() => (sel = i)}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => void choose(it)}
          >
            <span class="label">{@render marked(it.label, it.labelPos)}</span>
            {#if it.badge}<span class="badge">{it.badge}</span>{/if}
            {#if it.detail}<span class="detail">{@render marked(it.detail, it.detailPos ?? [])}</span>{/if}
          </div>
        {:else}
          <div class="empty">{filesError && palette.mode === 'files' ? filesError : query.trim() ? 'Nessun risultato' : 'Nessun elemento'}</div>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .cover { position: fixed; inset: 0; z-index: 200; display: flex; justify-content: center; align-items: flex-start; padding-top: 12vh; background: rgba(10, 12, 18, 0.35); }
  .panel { width: min(640px, 92vw); max-height: 60vh; display: flex; flex-direction: column; overflow: hidden; animation: pop var(--dur) ease; }
  @keyframes pop { from { opacity: 0; transform: translateY(-6px) scale(0.985); } to { opacity: 1; transform: none; } }
  .q { font: inherit; font-size: 15px; color: var(--txt); background: transparent; border: 0; border-bottom: 1px solid var(--edge-soft); padding: 14px 16px; outline: none; }
  .q::placeholder { color: var(--faint); }
  .list { overflow-y: auto; padding: 6px; }
  .sec { padding: 8px 10px 4px; font-size: 11px; letter-spacing: 0.04em; text-transform: uppercase; color: var(--faint); }
  .opt { display: flex; flex-wrap: wrap; align-items: baseline; gap: 2px 10px; padding: 7px 10px; border-radius: 8px; cursor: pointer; font-size: 13px; color: var(--txt); }
  .opt.on { background: var(--glass2); box-shadow: inset 0 0 0 1px var(--edge-soft); }
  .label { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 100%; }
  .detail { flex: 1 1 100%; font-size: 12px; color: var(--dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .badge { font-size: 11px; color: var(--faint); }
  mark { background: none; color: var(--accent); font-weight: 650; }
  .err { padding: 8px 16px; font-size: 12px; color: var(--wait); }
  .empty { padding: 18px 12px; text-align: center; color: var(--faint); font-size: 13px; }
</style>
