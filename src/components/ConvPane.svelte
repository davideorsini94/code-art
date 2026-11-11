<script lang="ts">
  import { onMount, tick } from 'svelte'
  import ChatItem from './ChatItem.svelte'
  import TodoStrip from './TodoStrip.svelte'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { chat } from '../lib/stores/chat.svelte'
  import { convMarkViewed } from '../lib/ipc'
  import { buildRows } from '../lib/conv/reduce'
  import { relativeTime } from '../lib/conv/time'
  import { WINDOW_THRESHOLD } from '../lib/conv/order'
  import { variableRange } from '../lib/conv/window'

  /** Altezza stimata di una riga non ancora misurata. */
  const ESTIMATE = 72
  const STICK_PX = 48

  const id = $derived(conversations.activeId)
  const conv = $derived(conversations.all.find((c) => c.sessionId === id))
  const cs = $derived(id ? chat.states[id] : undefined)
  const rows = $derived(cs ? buildRows(cs.items) : [])

  let now = $state(Date.now())
  let focused = $state(typeof document === 'undefined' ? true : document.hasFocus())
  let scroller = $state<HTMLDivElement>()
  let scrollTop = $state(0)
  let viewport = $state(600)
  let stick = true
  // altezze misurate (non reattive di per sé: `ver` si incrementa al massimo una volta per frame)
  const heights = new Map<string, number>()
  let ver = $state(0)
  let measuring = false

  onMount(() => {
    void chat.start()
    const tick = setInterval(() => (now = Date.now()), 60_000)
    const on = () => (focused = true)
    const off = () => (focused = false)
    window.addEventListener('focus', on)
    window.addEventListener('blur', off)
    return () => {
      chat.stop()
      clearInterval(tick)
      window.removeEventListener('focus', on)
      window.removeEventListener('blur', off)
    }
  })

  // carica la storia quando cambia la conversazione mostrata
  $effect(() => {
    if (id) {
      stick = true
      void chat.open(id)
    }
  })

  // "vista": la conversazione è sullo schermo con la finestra attiva (anche quando cambia lo stato, es. torna `waiting`)
  $effect(() => {
    const sid = id
    void conv?.status
    if (sid && focused) void convMarkViewed(sid).catch(() => {})
  })

  const virtual = $derived(rows.length > WINDOW_THRESHOLD)
  const hs = $derived.by(() => {
    void ver
    return rows.map((r) => heights.get(r.item.id) ?? ESTIMATE)
  })
  const range = $derived(virtual ? variableRange(hs, scrollTop, viewport) : { start: 0, end: rows.length, top: 0, bottom: 0 })
  const visible = $derived(rows.slice(range.start, range.end))

  // resta agganciato in fondo mentre arriva testo, a meno che l'utente sia salito
  $effect(() => {
    void cs
    void ver
    if (!stick || !scroller) return
    void tick().then(() => {
      if (scroller) scroller.scrollTop = scroller.scrollHeight
    })
  })

  function onscroll(e: Event & { currentTarget: HTMLDivElement }) {
    const el = e.currentTarget
    scrollTop = el.scrollTop
    stick = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX
  }

  function frameBump() {
    if (measuring) return
    measuring = true
    requestAnimationFrame(() => {
      measuring = false
      ver++
    })
  }

  function measureRow(node: HTMLElement, key: string) {
    let k = key
    const ro = new ResizeObserver(() => {
      const h = node.offsetHeight
      if (h && heights.get(k) !== h) {
        heights.set(k, h)
        if (virtual) frameBump()
      }
    })
    ro.observe(node)
    return { update: (next: string) => (k = next), destroy: () => ro.disconnect() }
  }

  function measureViewport(node: HTMLElement) {
    const ro = new ResizeObserver(() => (viewport = node.clientHeight))
    ro.observe(node)
    viewport = node.clientHeight
    return { destroy: () => ro.disconnect() }
  }

  const dotClass = $derived(conv ? { working: 'work', waiting: 'wait', idle: 'done' }[conv.status] : 'done')
</script>

<div class="pane">
  {#if !id || !conv}
    <p class="empty">Scegli una conversazione dall'elenco o creane una nuova.</p>
  {:else}
    <header>
      <div class="chatH">
        <span class="dot {dotClass}"></span>
        <span class="title">{conv.title}</span>
        {#if cs?.contextUsedPct != null}<span class="ctx" title="Contesto usato">{Math.round(cs.contextUsedPct)}%</span>{/if}
      </div>
      {#if conv.prevTitle}
        <div class="was">
          titolo aggiornato {conv.titleChangedAt ? relativeTime(conv.titleChangedAt, now) : ''} · prima: «{conv.prevTitle}»
        </div>
      {/if}
    </header>

    <div class="list" bind:this={scroller} use:measureViewport {onscroll} role="log" aria-label="Messaggi">
      {#if chat.error && !cs}
        <p class="err" role="alert">{chat.error}</p>
      {:else if !cs}
        <p class="empty">Carico la conversazione…</p>
      {:else if rows.length === 0}
        <p class="empty">Nessun messaggio.</p>
      {:else}
        {#if virtual}<div style:height="{range.top}px" aria-hidden="true"></div>{/if}
        {#each visible as row (row.item.id)}
          <div class="r" use:measureRow={row.item.id}><ChatItem {row} sessionId={id} /></div>
        {/each}
        {#if virtual}<div style:height="{range.bottom}px" aria-hidden="true"></div>{/if}
      {/if}
    </div>

    <div class="bottom">
      {#if cs}<TodoStrip todos={cs.todos} />{/if}
      <!-- SLOT COMPOSITORE (Task 11): il campo di risposta va qui, sotto la TodoStrip. -->
      <div class="composer-slot" data-slot="composer"></div>
    </div>
  {/if}
</div>

<style>
  .pane { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  header { flex: none; }
  .chatH { font-size: 16px; font-weight: 650; letter-spacing: -0.01em; display: flex; align-items: center; gap: 9px; }
  .title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ctx { margin-left: auto; font-size: 10.5px; font-weight: 400; color: var(--faint); }
  .was { color: var(--faint); font-size: 11px; margin: 2px 0 0 17px; }
  .list { flex: 1; min-height: 0; overflow-y: auto; margin: 10px -4px 0; padding: 0 4px; }
  .r { padding-bottom: 9px; }
  .bottom { flex: none; display: flex; flex-direction: column; gap: 8px; padding-top: 8px; }
  .composer-slot:empty { display: none; }
  .empty { color: var(--faint); text-align: center; margin-top: 24px; }
  .err { color: #ff8f8f; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
</style>
