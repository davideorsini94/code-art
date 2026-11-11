<script lang="ts" module>
  /** Istante dell'ultima scelta di modello riuscita, per sessione (sopravvive allo smontaggio del compositore). */
  let modelSetAt = $state<Record<string, number>>({})
</script>

<script lang="ts">
  import { onDestroy, tick } from 'svelte'
  import { chat } from '../lib/stores/chat.svelte'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { projectStore } from '../lib/stores/project.svelte'
  import { convInterrupt, convSaveImage, convSend, convSetModel, convSetPermissionMode } from '../lib/ipc'
  import { errorText } from '../lib/errors'
  import { BUILTIN_COMMANDS, completeSlash, slashMatches } from '../lib/conv/slash'
  import { displayedModel, modelLabel, modelOptions } from '../lib/conv/model'
  import { tooltip } from '../lib/tooltip'
  import { clearDraft, loadDraft, saveDraft } from '../lib/conv/draft'
  import { MAX_IMAGES, MAX_IMAGE_BYTES, MAX_TOTAL_BYTES, PATH_DRAG_TYPE, imageExt, mentionFor } from '../lib/conv/paths'
  import { PERMISSION_MODES as MODES, UNTRUSTED_HINT } from '../lib/conv/labels'
  import type { Conversation } from '../lib/types'

  let { conv }: { conv: Conversation } = $props()

  type Thumb = { url: string; bytes: Uint8Array; ext: string; name: string; path?: string }

  const sessionId = $derived(conv.sessionId)
  const working = $derived(conv.status === 'working')
  const untrusted = $derived(projectStore.untrusted.length > 0)
  const init = $derived(conversations.inits[sessionId])
  const commands = $derived(init?.slashCommands?.length ? init.slashCommands : BUILTIN_COMMANDS)
  const currentModel = $derived(displayedModel({ convModel: conv.model, initModel: init?.model ?? null, lastSetAt: modelSetAt[sessionId] ?? null, initAt: init?.at ?? null }))
  const models = $derived(modelOptions(currentModel))
  const modes = $derived(MODES.some(([m]) => m === conv.permissionMode) ? MODES : [...MODES, [conv.permissionMode, conv.permissionMode] as [string, string]])
  const pct = $derived(conv.contextUsedPct == null ? null : Math.max(0, Math.min(100, Math.round(conv.contextUsedPct))))

  // un compositore per conversazione (ConvPane lo monta dentro `{#key}`): la bozza si legge una volta
  // svelte-ignore state_referenced_locally
  let text = $state(loadDraft(sessionId))
  let cursor = $state(0)
  let thumbs = $state<Thumb[]>([])
  let error = $state<string | null>(null)
  let busy = $state(false)
  let pick = $state(0)
  let popupOff = $state(false)
  let area = $state<HTMLTextAreaElement>()

  const matches = $derived(popupOff ? [] : slashMatches(text, cursor, commands))
  $effect(() => {
    void matches
    pick = 0
  })

  $effect(() => {
    saveDraft(sessionId, text)
  })

  // altezza del campo: cresce col testo fino a un massimo
  $effect(() => {
    void text
    if (!area) return
    area.style.height = 'auto'
    area.style.height = `${Math.min(area.scrollHeight, 160)}px`
  })

  let alive = true
  onDestroy(() => {
    alive = false
    thumbs.forEach((t) => URL.revokeObjectURL(t.url))
  })

  const syncCursor = () => {
    cursor = area?.selectionStart ?? text.length
    popupOff = false
  }

  async function setCursor(pos: number) {
    await tick()
    area?.setSelectionRange(pos, pos)
    cursor = pos
    area?.focus()
  }

  function choose(cmd: string) {
    const r = completeSlash(text, cursor, cmd)
    text = r.text
    popupOff = true
    void setCursor(r.cursor)
  }

  function insertAtCursor(s: string) {
    const at = area?.selectionStart ?? text.length
    const end = area?.selectionEnd ?? at
    const pre = text.slice(0, at)
    const lead = pre && !/\s$/.test(pre) ? ' ' : ''
    text = `${pre}${lead}${s} ${text.slice(end)}`
    void setCursor(pre.length + lead.length + s.length + 1)
  }

  /** Controlli di numero e dimensione sulle miniature attuali; ritorna il messaggio d'errore o null. */
  function imageLimit(size: number): string | null {
    if (thumbs.length >= MAX_IMAGES) return `al massimo ${MAX_IMAGES} immagini per messaggio`
    if (size > MAX_IMAGE_BYTES) return 'immagine troppo grande (massimo 20 MB)'
    if (thumbs.reduce((n, t) => n + t.bytes.length, 0) + size > MAX_TOTAL_BYTES) return 'immagini troppo grandi: al massimo 30 MB per messaggio'
    return null
  }

  async function addImage(file: File) {
    const ext = imageExt(file.type)
    if (!ext) return void (error = `formato non supportato: ${file.type || file.name}`)
    const early = imageLimit(file.size)
    if (early) return void (error = early)
    const bytes = new Uint8Array(await file.arrayBuffer())
    if (!alive) return
    const late = imageLimit(bytes.length)
    if (late) return void (error = late)
    error = null
    thumbs = [...thumbs, { url: URL.createObjectURL(new Blob([bytes], { type: file.type })), bytes, ext, name: file.name || `immagine.${ext}` }]
  }

  function onpaste(ev: ClipboardEvent) {
    const files = [...(ev.clipboardData?.files ?? [])].filter((f) => f.type.startsWith('image/'))
    if (!files.length) return
    ev.preventDefault()
    for (const f of files) void addImage(f)
  }

  const accepts = (ev: DragEvent) => !!ev.dataTransfer && [...ev.dataTransfer.types].some((t) => t === PATH_DRAG_TYPE || t === 'Files')

  function ondragover(ev: DragEvent) {
    if (!accepts(ev)) return
    ev.preventDefault()
    if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'copy'
  }

  function ondrop(ev: DragEvent) {
    if (!accepts(ev)) return
    ev.preventDefault()
    const path = ev.dataTransfer?.getData(PATH_DRAG_TYPE)
    if (path) insertAtCursor(mentionFor(path, projectStore.project?.folders ?? []))
    for (const f of ev.dataTransfer?.files ?? []) void addImage(f)
  }

  function removeThumb(t: Thumb) {
    URL.revokeObjectURL(t.url)
    thumbs = thumbs.filter((x) => x !== t)
  }

  const canSend = $derived(!working && !busy && !untrusted && (text.trim() !== '' || thumbs.length > 0))

  async function send() {
    if (!canSend) return
    busy = true
    error = null
    const sent = text
    const sentThumbs = [...thumbs]
    try {
      for (const t of sentThumbs) t.path ??= await convSaveImage(t.bytes, t.ext)
      const paths = sentThumbs.map((t) => t.path as string)
      await convSend(sessionId, sent, paths)
      chat.addUser(sessionId, sent, paths)
      sentThumbs.forEach((t) => URL.revokeObjectURL(t.url))
      thumbs = thumbs.filter((t) => !sentThumbs.includes(t))
      // quanto scritto nel frattempo non si cancella
      if (text === sent) {
        text = ''
        clearDraft(sessionId)
      }
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  async function run(op: () => Promise<void>) {
    error = null
    try {
      await op()
    } catch (e) {
      error = errorText(e)
    }
  }
  const interrupt = () => run(() => convInterrupt(sessionId))
  /** Se la scelta fallisce il `<select>` torna al valore reale, così non contraddice il chip. */
  async function guarded(el: HTMLSelectElement, actual: string, op: () => Promise<void>) {
    error = null
    try {
      await op()
    } catch (e) {
      error = errorText(e)
      el.value = actual
    }
  }
  const setModel = (el: HTMLSelectElement) =>
    guarded(el, currentModel, async () => {
      await convSetModel(sessionId, el.value)
      modelSetAt = { ...modelSetAt, [sessionId]: Date.now() }
    })
  const setMode = (el: HTMLSelectElement) => guarded(el, conv.permissionMode, () => convSetPermissionMode(sessionId, el.value))

  function onkeydown(ev: KeyboardEvent) {
    if (ev.isComposing || ev.keyCode === 229) return
    if (matches.length) {
      if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
        ev.preventDefault()
        pick = (pick + (ev.key === 'ArrowDown' ? 1 : matches.length - 1)) % matches.length
        return
      }
      if ((ev.key === 'Enter' || ev.key === 'Tab') && !ev.metaKey && !ev.ctrlKey) {
        ev.preventDefault()
        return choose(matches[pick])
      }
      if (ev.key === 'Escape') {
        ev.preventDefault()
        popupOff = true
        return
      }
    }
    if (ev.key === 'Enter' && (ev.metaKey || ev.ctrlKey)) {
      ev.preventDefault()
      void send()
    } else if (ev.key === 'Escape' && working) {
      ev.preventDefault()
      void interrupt()
    }
  }

  const sendTip = $derived<string | { text: string; shortcut: string }>(
    untrusted ? UNTRUSTED_HINT : canSend ? { text: 'Invia il messaggio', shortcut: '⌘↵' } : busy ? 'Invio in corso…' : 'Scrivi un messaggio per inviare',
  )
  const modeLabel = $derived(modes.find(([m]) => m === conv.permissionMode)?.[1] ?? conv.permissionMode)
  const R = 8
  const C = 2 * Math.PI * R
</script>

<div class="composer" role="group" aria-label="Rispondi a Claude" {ondragover} {ondrop}>
  {#if conv.maybeOpenElsewhere}
    <p class="warn" role="note">Questa conversazione sembra aperta anche in un altro programma: le risposte potrebbero mescolarsi.</p>
  {/if}
  {#if thumbs.length}
    <ul class="thumbs" aria-label="Immagini allegate">
      {#each thumbs as t (t)}
        <li>
          <img src={t.url} alt={t.name} use:tooltip={t.name} />
          <button type="button" class="x" aria-label="Rimuovi {t.name}" use:tooltip={`Rimuovi ${t.name}`} onclick={() => removeThumb(t)}>×</button>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="box">
    {#if matches.length}
      <ul class="pop" role="listbox" aria-label="Comandi">
        {#each matches as m, i (m)}
          <li role="option" aria-selected={i === pick} class:on={i === pick} data-no-tooltip="voce di un elenco di completamento: il testo è già la spiegazione">
            <button type="button" tabindex="-1" data-no-tooltip="voce di un elenco di completamento: il testo è già la spiegazione" onmousedown={(e) => e.preventDefault()} onclick={() => choose(m)}>/{m}</button>
          </li>
        {/each}
      </ul>
    {/if}
    <textarea
      bind:this={area}
      bind:value={text}
      rows="1"
      placeholder={working ? 'Claude sta lavorando — puoi scrivere, invierai a fine turno' : 'Rispondi a Claude…'}
      aria-label="Messaggio"
      {onkeydown}
      {onpaste}
      oninput={syncCursor}
      onclick={syncCursor}
      onkeyup={(e) => !['ArrowDown', 'ArrowUp', 'Enter', 'Tab', 'Escape'].includes(e.key) && syncCursor()}
    ></textarea>
    <span class="hint" use:tooltip={{ text: 'Invia il messaggio', shortcut: '⌘↵' }}>⌘↵</span>
  </div>

  {#if error}<p class="err" role="alert">{error}</p>{:else if untrusted}<p class="ro">{UNTRUSTED_HINT}</p>{/if}

  <div class="bar">
    <label class="chip mode" class:bypass={conv.permissionMode === 'bypassPermissions'}>
      <span class="sr">Modalità permessi</span>
      <select value={conv.permissionMode} use:tooltip={`Modalità permessi: ${modeLabel}`} onchange={(e) => setMode(e.currentTarget)}>
        {#each modes as [m, label] (m)}<option value={m}>{label}</option>{/each}
      </select>
    </label>
    <label class="chip">
      <span class="sr">Modello</span>
      <select value={currentModel} use:tooltip={`Modello: ${currentModel ? modelLabel(currentModel) : 'nessuno'}`} onchange={(e) => setModel(e.currentTarget)}>
        {#if !currentModel}<option value="" disabled>Modello</option>{/if}
        {#each models as m (m)}<option value={m}>{modelLabel(m)}</option>{/each}
      </select>
    </label>
    <span class="grow"></span>
    {#if pct !== null}
      <span class="ring" use:tooltip={`Contesto usato: ${pct}%`} role="img" aria-label="Contesto usato: {pct}%">
        <svg width="22" height="22" viewBox="0 0 22 22" aria-hidden="true">
          <circle cx="11" cy="11" r={R} class="bg" />
          <circle cx="11" cy="11" r={R} class="fg" class:hot={pct >= 85} stroke-dasharray={C} stroke-dashoffset={C * (1 - pct / 100)} transform="rotate(-90 11 11)" />
        </svg>
        <span>{pct}%</span>
      </span>
    {/if}
    {#if working}
      <button type="button" class="btn stop" use:tooltip={{ text: 'Interrompi', shortcut: 'Esc' }} onclick={interrupt}>Interrompi</button>
      <span class="tipw" use:tooltip={'Attendi la fine del turno per inviare'}><button type="button" class="btn" disabled data-no-tooltip="coperto dallo span esterno">Invia</button></span>
    {:else}
      <span class="tipw" use:tooltip={sendTip}><button type="button" class="btn go" disabled={!canSend} data-no-tooltip="coperto dallo span esterno" onclick={send}>Invia</button></span>
    {/if}
  </div>
</div>

<style>
  .composer { display: flex; flex-direction: column; gap: 6px; }
  .box { position: relative; display: flex; align-items: flex-end; gap: 8px; padding: 9px 12px; border-radius: 14px; background: var(--well); border: 1px solid var(--edge-soft); }
  .box:focus-within { border-color: var(--accent); }
  textarea { flex: 1; min-width: 0; resize: none; background: none; border: 0; outline: 0; color: var(--txt); font: inherit; line-height: 1.4; max-height: 160px; }
  textarea::placeholder { color: var(--faint); }
  .hint { color: var(--faint); font-size: 11px; padding-bottom: 2px; }
  .pop { position: absolute; left: 0; right: 0; bottom: calc(100% + 6px); list-style: none; margin: 0; padding: 4px; border-radius: 12px; background: var(--surface-pop); border: 1px solid var(--edge-soft); z-index: 5; }
  .pop button { display: block; width: 100%; text-align: left; background: none; border: 0; color: var(--txt); font: inherit; padding: 5px 9px; border-radius: 8px; cursor: pointer; }
  .pop li.on button { background: var(--glass2); }
  .thumbs { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 6px; }
  .thumbs li { position: relative; }
  .thumbs img { display: block; height: 52px; max-width: 96px; object-fit: cover; border-radius: 8px; border: 1px solid var(--edge-soft); }
  .x { position: absolute; top: -6px; right: -6px; width: 18px; height: 18px; line-height: 14px; padding: 0; border-radius: 50%; border: 1px solid var(--edge-soft); background: var(--surface-2); color: var(--txt); cursor: pointer; }
  .bar { display: flex; align-items: center; gap: 6px; }
  .grow { flex: 1; }
  .chip { display: inline-flex; border: 1px solid var(--edge-soft); border-radius: 999px; background: var(--well); color: var(--dim); font-size: 11.5px; }
  .chip select { appearance: none; background: none; border: 0; outline: 0; color: inherit; font: inherit; padding: 3px 10px; cursor: pointer; max-width: 180px; }
  .chip select option { color: var(--ink); }
  .chip:focus-within { border-color: var(--accent); }
  .chip.bypass { color: var(--wait); border-color: var(--wait); }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  .ring { display: inline-flex; align-items: center; gap: 4px; color: var(--faint); font-size: 10.5px; font-variant-numeric: tabular-nums; }
  .ring .bg { fill: none; stroke: var(--edge-soft); stroke-width: 3; }
  .ring .fg { fill: none; stroke: var(--work); stroke-width: 3; stroke-linecap: round; }
  .ring .fg.hot { stroke: var(--wait); }
  .btn { padding: 5px 14px; border-radius: 10px; border: 1px solid var(--edge-soft); background: var(--glass); color: var(--txt); font: inherit; font-size: 12px; cursor: pointer; }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }
  /* un pulsante disabilitato non riceve eventi del puntatore: il suggerimento sta sul contenitore */
  .tipw { display: inline-flex; }
  .tipw button:disabled { pointer-events: none; }
  .btn.go { background: var(--accent); color: var(--ink); border-color: transparent; font-weight: 600; }
  .btn.stop { border-color: var(--wait); color: var(--wait); }
  .err { color: var(--err); font-size: 11.5px; margin: 0; }
  .ro { color: var(--faint); font-size: 11.5px; margin: 0; }
  .warn { color: var(--wait); font-size: 11.5px; margin: 0; }
  @media (prefers-reduced-motion: no-preference) { .chip, .box { transition: border-color var(--dur); } }
</style>
