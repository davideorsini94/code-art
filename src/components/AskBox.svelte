<script lang="ts">
  import type { ChatItem } from '../lib/types'
  import { errorText } from '../lib/errors'
  import { multiAnswer, type Picked } from '../lib/conv/ask'
  import { chat } from '../lib/stores/chat.svelte'
  import { renderMarkdown } from '../lib/safe-markdown'
  import { links } from '../lib/conv/links'
  import { tooltip } from '../lib/tooltip'

  type Ask = Extract<ChatItem, { kind: 'ask' }>
  let { item, sessionId }: { item: Ask; sessionId: string } = $props()

  const WAIT = 'Risposta in corso…'
  let busy = $state(false)
  let error = $state<string | null>(null)
  let free = $state('')

  async function answer(choice: string) {
    if (busy || item.answered || !choice.trim()) return
    busy = true
    error = null
    try {
      await chat.answer(sessionId, item.requestId, choice)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  // più domande (o una a scelta multipla): una risposta per domanda, poi un solo invio
  const multi = $derived(item.askType === 'question' && (item.questions.length > 1 || !!item.questions[0]?.multiSelect))
  let picked = $state<Picked[]>([])
  $effect(() => {
    if (picked.length !== item.questions.length) picked = item.questions.map(() => ({ options: [], other: '' }))
  })
  const answers = $derived(multiAnswer(item.questions, picked))

  function toggle(qi: number, label: string, multiSelect: boolean) {
    const p = picked[qi]
    if (multiSelect) p.options = p.options.includes(label) ? p.options.filter((l) => l !== label) : [...p.options, label]
    else {
      p.options = p.options[0] === label ? [] : [label]
      p.other = ''
    }
  }
  function typed(qi: number, value: string, multiSelect: boolean) {
    picked[qi].other = value
    if (!multiSelect && value.trim()) picked[qi].options = []
  }

  const title = $derived(item.askType === 'plan' ? 'Claude propone un piano' : item.askType === 'question' ? 'Come procedo?' : 'Claude chiede il permesso')
</script>

<div class="ask" class:done={item.answered} role="group" aria-label="Claude ha bisogno di te">
  <b>{title}</b>
  {#if item.answered}<span class="tag">risposto</span>{:else}<div class="sub">Serve una tua scelta per continuare.</div>{/if}

  {#if item.askType === 'plan'}
    <div class="md plan" use:links>{@html renderMarkdown(item.prompt)}</div>
  {:else}
    <p class="prompt">{item.prompt}</p>
  {/if}

  {#if !item.answered}
    <div class="btns">
      {#if item.askType === 'permission'}
        <button class="btn p" use:tooltip={busy ? WAIT : 'Consenti a Claude di procedere'} aria-disabled={busy} onclick={() => answer('allow')}>Consenti</button>
        <button class="btn" use:tooltip={busy ? WAIT : 'Rifiuta: Claude non eseguirà l\'azione'} aria-disabled={busy} onclick={() => answer('deny')}>Nega</button>
      {:else if item.askType === 'plan'}
        <button class="btn p" use:tooltip={busy ? WAIT : 'Approva il piano: Claude lo esegue'} aria-disabled={busy} onclick={() => answer('approve')}>Approva piano</button>
        <button class="btn" use:tooltip={busy ? WAIT : 'Rifiuta il piano: Claude continua a pianificare'} aria-disabled={busy} onclick={() => answer('reject')}>Continua a pianificare</button>
      {:else if !multi}
        {#each item.options as opt, i (i)}
          <button class="btn" class:p={i === 0} use:tooltip={busy ? WAIT : `Rispondi: ${opt}`} aria-disabled={busy} onclick={() => answer(opt)}>{opt}</button>
        {/each}
      {/if}
    </div>
    {#if multi}
      <form class="multi" onsubmit={(e) => { e.preventDefault(); if (answers) void answer(JSON.stringify(answers)) }}>
        {#each item.questions as q, qi (qi)}
          <fieldset class="q">
            <legend>{#if q.header}<span class="hdr">{q.header}</span>{/if}{q.question}</legend>
            <div class="btns">
              {#each q.options as o, oi (oi)}
                {@const on = picked[qi]?.options.includes(o.label)}
                <button type="button" class="btn" class:on role={q.multiSelect ? 'checkbox' : 'radio'} aria-checked={!!on}
                  use:tooltip={o.description || (q.multiSelect ? `Aggiungi o togli: ${o.label}` : `Scegli: ${o.label}`)}
                  aria-disabled={busy} onclick={() => !busy && toggle(qi, o.label, q.multiSelect)}>{q.multiSelect ? (on ? '☑ ' : '☐ ') : ''}{o.label}</button>
              {/each}
            </div>
            <input type="text" placeholder="Altro…" value={picked[qi]?.other ?? ''} disabled={busy} aria-label={`Altra risposta: ${q.question}`}
              oninput={(e) => typed(qi, e.currentTarget.value, q.multiSelect)} />
          </fieldset>
        {/each}
        <button class="btn p" type="submit" use:tooltip={busy ? WAIT : answers ? 'Invia le risposte a Claude' : 'Rispondi a tutte le domande per inviare'} aria-disabled={busy || !answers}>Invia risposte</button>
      </form>
    {:else if item.askType === 'question'}
      <form class="free" onsubmit={(e) => { e.preventDefault(); void answer(free) }}>
        <input type="text" placeholder="Oppure scrivi una risposta…" bind:value={free} disabled={busy} aria-label="Risposta libera" />
        <button class="btn" type="submit" use:tooltip={busy ? WAIT : free.trim() ? 'Invia la risposta scritta' : 'Scrivi una risposta per inviarla'} aria-disabled={busy || !free.trim()}>Invia</button>
      </form>
    {/if}
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .ask { border: 1px solid var(--wait-line); background: var(--wait-soft); border-radius: 13px; padding: 10px 12px; }
  .ask.done { opacity: 0.6; border-color: var(--edge-soft); background: var(--glass); }
  .sub { color: var(--dim); }
  .tag { color: var(--faint); font-size: 11px; margin-left: 8px; }
  .prompt { margin: 6px 0 0; white-space: pre-wrap; word-break: break-word; color: var(--txt); }
  .plan { margin-top: 6px; max-height: 320px; overflow: auto; }
  .btns { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
  .btn { padding: 4px 12px; border-radius: 999px; background: var(--glass2); border: 1px solid var(--edge); color: inherit; font-size: 11.5px; cursor: pointer; }
  .btn.p { background: var(--wait-strong); border-color: var(--wait-line); }
  .btn[aria-disabled='true'] { opacity: 0.5; cursor: default; }
  .multi { display: flex; flex-direction: column; gap: 8px; margin-top: 8px; }
  .q { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .q legend { padding: 0; color: var(--txt); }
  .hdr { color: var(--faint); font-size: 11px; margin-right: 6px; text-transform: uppercase; letter-spacing: 0.06em; }
  .q .btns { margin-top: 0; }
  .btn.on { border-color: var(--wait); background: var(--wait-mid); }
  .q input { padding: 4px 10px; border-radius: 999px; border: 1px solid var(--edge-soft); background: var(--well); color: inherit; font: inherit; font-size: 12px; }
  .multi > .btn { align-self: flex-start; }
  .free { display: flex; gap: 6px; margin-top: 8px; }
  .free input { flex: 1; min-width: 0; padding: 4px 10px; border-radius: 999px; border: 1px solid var(--edge-soft); background: var(--well); color: inherit; font: inherit; font-size: 12px; }
  .err { color: var(--err); font-size: 11.5px; margin: 6px 0 0; }
  @media (prefers-reduced-motion: no-preference) { .ask { transition: opacity var(--dur); } }
</style>
