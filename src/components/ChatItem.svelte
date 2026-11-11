<script lang="ts">
  import ChatItem from './ChatItem.svelte'
  import AskBox from './AskBox.svelte'
  import PlainBlock from './PlainBlock.svelte'
  import { renderMarkdown } from '../lib/safe-markdown'
  import { links } from '../lib/conv/links'
  import { diffAction, isSubagent, prettyJson, toolSummary } from '../lib/conv/tools'
  import { editor } from '../lib/stores/editor.svelte'
  import { tooltip } from '../lib/tooltip'
  import type { Row } from '../lib/conv/reduce'
  import { ERROR_LABEL, INTERRUPTED_LABEL } from '../lib/types'
  import { chat } from '../lib/stores/chat.svelte'
  import { projectStore } from '../lib/stores/project.svelte'
  import { convSend } from '../lib/ipc'
  import { errorText } from '../lib/errors'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { UNTRUSTED_HINT } from '../lib/conv/labels'
  import { untrack } from 'svelte'

  let { row, sessionId }: { row: Row; sessionId: string } = $props()
  const item = $derived(row.item)

  let open = $state(false)

  // Testo del messaggio passato al markdown: durante lo streaming (conversazione `working`) marked + DOMPurify sul
  // testo intero girano al massimo ogni RENDER_MS (l'ultimo testo arriva sempre); altrimenti subito.
  const RENDER_MS = 100
  let shown = $state(untrack(() => (item.kind === 'assistant' ? item.text : '')))
  let shownAt = 0
  $effect(() => {
    if (item.kind !== 'assistant') return
    const text = item.text
    const working = conversations.all.find((c) => c.sessionId === sessionId)?.status === 'working'
    if (text === untrack(() => shown)) return
    const show = () => {
      shownAt = Date.now()
      shown = text
    }
    const wait = working ? RENDER_MS - (Date.now() - shownAt) : 0
    if (wait <= 0) return show()
    const timer = setTimeout(show, wait)
    return () => clearTimeout(timer)
  })
  const imagesNote = $derived(item.kind === 'user' && item.images.length ? (item.images.length === 1 ? '1 immagine allegata' : `${item.images.length} immagini allegate`) : null)

  const action = $derived(item.kind === 'tool' ? diffAction(item.name, item.input, item.filePath) : null)
  const steps = $derived(row.children.filter((c) => c.kind === 'tool').length)

  function openDiff() {
    if (!action) return
    if (action.type === 'diff') editor.openDiff(action.path, action.before, action.after)
    else void editor.open(action.path)
  }

  // "Turno interrotto": riprende inviando "Continua" come farebbe il compositore
  const RESUME_TEXT = 'Continua'
  let resuming = $state(false)
  let resumeError = $state<string | null>(null)
  const untrusted = $derived(projectStore.untrusted.length > 0)
  const busy = $derived(conversations.all.find((c) => c.sessionId === sessionId)?.status === 'working')
  async function resume() {
    if (resuming || untrusted || busy) return
    resuming = true
    resumeError = null
    try {
      await convSend(sessionId, RESUME_TEXT, [])
      chat.addUser(sessionId, RESUME_TEXT)
    } catch (e) {
      resumeError = errorText(e)
    } finally {
      resuming = false
    }
  }

  const mark = (ok: boolean | null) => (ok === null ? '…' : ok ? '✓' : '✗')
</script>

{#if item.kind === 'user'}
  <div class="msg me"><span class="txt">{item.text}</span>{#if imagesNote}<span class="imgs">{imagesNote}</span>{/if}</div>
{:else if item.kind === 'assistant'}
  <div class="msg ai md" use:links>{@html renderMarkdown(shown)}</div>
{:else if item.kind === 'thinking'}
  <div class="think">
    <button class="head" use:tooltip={open ? 'Nascondi il ragionamento' : 'Mostra il ragionamento di Claude'} aria-expanded={open} onclick={() => (open = !open)}>{open ? '▾' : '▸'} Ragionamento</button>
    {#if open}{#if item.text}<PlainBlock text={item.text} />{:else}<span class="faint">Il testo del ragionamento non è disponibile.</span>{/if}{/if}
  </div>
{:else if item.kind === 'tool' && isSubagent(item.name)}
  <div class="sub" class:err={item.ok === false}>
    <button class="head" use:tooltip={`${open ? 'Nascondi' : 'Mostra'} i passi del subagent: ${toolSummary(item.name, item.input)}`} aria-expanded={open} onclick={() => (open = !open)}>
      <span class="caret">{open ? '▾' : '▸'}</span>
      <span class="name">Subagent</span>
      <span class="arg">{toolSummary(item.name, item.input)}</span>
      <span class="meta">{steps === 1 ? '1 passo' : `${steps} passi`} {mark(item.ok)}</span>
    </button>
    {#if open}
      <div class="body">
        {#each row.children as c (c.id)}
          <ChatItem row={{ item: c, children: [] }} {sessionId} />
        {/each}
        {#if item.result}<PlainBlock text={item.result} label="Risultato" />{/if}
      </div>
    {/if}
  </div>
{:else if item.kind === 'tool'}
  <div class="tool" class:err={item.ok === false}>
    <div class="line">
      <button class="head" use:tooltip={`${open ? 'Nascondi' : 'Mostra'} input e risultato: ${item.name} ${toolSummary(item.name, item.input)}`} aria-expanded={open} onclick={() => (open = !open)}>
        <span class="caret">{open ? '▾' : '▸'}</span>
        <span class="name">{item.name}</span>
        <span class="arg">{toolSummary(item.name, item.input)}</span>
        <span class="meta">{mark(item.ok)}</span>
      </button>
      {#if action}<button class="diff" use:tooltip={action.type === 'diff' ? 'Apri le modifiche nell\'editor (diff)' : 'Apri il file nell\'editor'} onclick={openDiff}>{action.type === 'diff' ? 'Apri diff' : 'Apri file'}</button>{/if}
    </div>
    {#if open}
      <div class="body">
        <PlainBlock text={prettyJson(item.input)} label="Input" />
        {#if item.result !== null}<PlainBlock text={item.result} label="Risultato" />{/if}
      </div>
    {/if}
  </div>
{:else if item.kind === 'ask'}
  <AskBox {item} {sessionId} />
{:else if item.kind === 'raw' && item.label === INTERRUPTED_LABEL}
  <div class="interrupted" role="status">
    <span>Il turno è stato interrotto.</span>
    <button class="diff" use:tooltip={untrusted ? UNTRUSTED_HINT : resuming ? 'Invio in corso…' : 'Chiedi a Claude di continuare il turno'} aria-disabled={untrusted || resuming || busy} onclick={resume}>Riprendi</button>
    {#if resumeError}<span class="errtxt" role="alert">{resumeError}</span>{/if}
  </div>
{:else if item.kind === 'raw' && item.label === ERROR_LABEL}
  <div class="errnote" role="alert"><b>{ERROR_LABEL}</b><span class="txt">{item.json}</span></div>
{:else if item.kind === 'raw'}
  <div class="tool raw">
    <button class="head" use:tooltip={`${open ? 'Nascondi' : 'Mostra'} ${item.label}`} aria-expanded={open} onclick={() => (open = !open)}><span class="caret">{open ? '▾' : '▸'}</span><span class="name">{item.label}</span></button>
    {#if open}<div class="body"><PlainBlock text={item.json} /></div>{/if}
  </div>
{/if}

<style>
  .msg { padding: 9px 12px; border-radius: 13px; max-width: 92%; overflow-wrap: anywhere; }
  .me { background: var(--accent-soft); border: 1px solid var(--accent-line); margin-left: auto; }
  .txt { white-space: pre-wrap; }
  .imgs { display: block; margin-top: 4px; color: var(--faint); font-size: 11px; }
  .ai { background: var(--glass); border: 1px solid var(--edge-soft); }
  .tool, .sub, .think { font: 11.5px ui-monospace, 'SF Mono', monospace; color: var(--dim); border-radius: 9px; background: var(--well); padding: 2px 10px; }
  .sub { border: 1px solid var(--edge-soft); }
  .err .meta { color: var(--err); }
  .line { display: flex; align-items: center; gap: 8px; }
  .head { display: flex; flex: 1; min-width: 0; align-items: baseline; gap: 8px; background: none; border: 0; color: inherit; font: inherit; cursor: pointer; padding: 5px 0; text-align: left; }
  .caret { flex: none; width: 10px; color: var(--faint); }
  .name { flex: none; color: var(--txt); }
  .arg { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { flex: none; margin-left: auto; color: var(--work); }
  .diff { flex: none; background: var(--glass2); border: 1px solid var(--edge); color: var(--txt); border-radius: 999px; font-size: 10.5px; padding: 1px 9px; cursor: pointer; }
  .body { padding: 0 0 8px 18px; }
  .faint { color: var(--faint); }
  .interrupted { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 7px 12px; border-radius: 11px; border: 1px solid var(--wait); color: var(--wait); font-size: 12px; }
  .interrupted .diff[aria-disabled='true'] { opacity: 0.5; cursor: default; }
  .errtxt { color: var(--err); }
  .errnote { display: flex; flex-direction: column; gap: 3px; padding: 8px 12px; border-radius: 11px; border: 1px solid var(--err); background: var(--err-soft); color: var(--err); overflow-wrap: anywhere; }
  .errnote .txt { color: var(--txt); }
</style>
