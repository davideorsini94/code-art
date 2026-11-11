<script lang="ts">
  import { capText } from '../lib/conv/tools'
  import { tooltip } from '../lib/tooltip'

  /** Testo non fidato mostrato come testo (mai HTML); oltre 20 000 caratteri solo l'inizio, con "mostra tutto". */
  let { text, label = '' }: { text: string; label?: string } = $props()
  let all = $state(false)
  const shown = $derived(all ? { text, truncated: false } : capText(text))
</script>

{#if label}<div class="lbl">{label}</div>{/if}
<pre class="plain">{shown.text}</pre>
{#if shown.truncated}
  <button class="more" use:tooltip={'Mostra l\'intero testo'} onclick={() => (all = true)}>mostra tutto ({text.length.toLocaleString('it-IT')} caratteri)</button>
{/if}

<style>
  .lbl { font-size: 10.5px; color: var(--faint); margin: 6px 0 2px; }
  .plain {
    font: 11.5px/1.5 ui-monospace, 'SF Mono', monospace;
    color: var(--dim);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    max-height: 320px;
    overflow: auto;
  }
  .more { background: none; border: 0; color: var(--accent); font-size: 11px; cursor: pointer; padding: 2px 0; }
</style>
