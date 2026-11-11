<script lang="ts">
  import { onMount } from 'svelte'
  import { claudeCheck } from '../lib/ipc'

  let { onresult }: { onresult?: (found: boolean) => void } = $props()

  // Copre la colonna conversazioni finché `claude` non viene trovato nel PATH della shell di login.
  let missing = $state(false)
  let checking = $state(false)
  let copied = $state<string | null>(null)

  const steps = [
    { label: 'Installa Claude Code', cmd: 'curl -fsSL https://claude.ai/install.sh | bash' },
    { label: 'Oppure con npm', cmd: 'npm install -g @anthropic-ai/claude-code' },
    { label: 'Poi accedi (segui le istruzioni)', cmd: 'claude' },
  ]

  async function check() {
    checking = true
    try {
      missing = !(await claudeCheck()).found
    } catch {
      missing = true
    } finally {
      onresult?.(!missing)
      checking = false
    }
  }

  async function copy(cmd: string) {
    try {
      await navigator.clipboard.writeText(cmd)
      copied = cmd
      setTimeout(() => (copied = copied === cmd ? null : copied), 1500)
    } catch {
      copied = null
    }
  }

  onMount(() => void check())
</script>

{#if missing}
  <div class="missing" role="alert" aria-label="Claude Code non trovato">
    <h2>Claude Code non trovato</h2>
    <p>Per usare le conversazioni serve la CLI <code>claude</code>. Eseguila nel terminale:</p>
    {#each steps as s (s.cmd)}
      <div class="step">
        <span class="label">{s.label}</span>
        <div class="cmd">
          <code>{s.cmd}</code>
          <button onclick={() => copy(s.cmd)}>{copied === s.cmd ? 'Copiato' : 'Copia'}</button>
        </div>
      </div>
    {/each}
    <button class="retry" disabled={checking} onclick={() => check()}>{checking ? 'Controllo…' : 'Riprova'}</button>
  </div>
{/if}

<style>
  .missing { display: flex; flex-direction: column; gap: 10px; height: 100%; overflow: auto; }
  h2 { margin: 0; font-size: 15px; }
  p { margin: 0; font-size: 12px; color: var(--faint); }
  .step { display: flex; flex-direction: column; gap: 4px; }
  .label { font-size: 11px; color: var(--faint); }
  .cmd { display: flex; gap: 6px; align-items: center; }
  code {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
    border-radius: 6px;
    background: rgba(0, 0, 0, 0.35);
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11px;
    word-break: break-all;
    user-select: text;
  }
  p code { padding: 1px 4px; flex: none; }
  button {
    flex: none;
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--edge-soft);
    background: var(--glass);
    color: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  button:hover:not(:disabled) { border-color: #8fa8ff; }
  button:disabled { opacity: 0.6; cursor: default; }
  .retry { align-self: flex-start; }
</style>
