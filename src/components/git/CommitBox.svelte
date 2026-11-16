<script lang="ts">
  import { tick } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { git } from '../../lib/stores/git.svelte'

  let { repo, stagedCount }: { repo: string; stagedCount: number } = $props()

  let message = $state('')
  let amend = $state(false)
  let field = $state<HTMLTextAreaElement>()

  const busy = $derived(git.busy !== null)
  const reason = $derived(
    message.trim() === '' ? 'Scrivi un messaggio di commit' : stagedCount === 0 && !amend ? 'Non c’è niente in stage: aggiungi dei file con ＋' : busy ? 'Un’operazione è in corso' : '',
  )

  async function commit() {
    if (reason) return
    if (await git.commit(repo, message, amend)) {
      message = ''
      amend = false
    }
  }

  // "Commit" dalla palette: il focus arriva sul campo
  $effect(() => {
    if (git.focusTick > 0 && git.primary === repo) void tick().then(() => field?.focus())
  })
</script>

<div class="box">
  <textarea
    bind:this={field}
    bind:value={message}
    rows="2"
    placeholder="Messaggio (⌘↵ per confermare)"
    aria-label="Messaggio di commit"
    onkeydown={(e) => {
      if (e.key === 'Enter' && e.metaKey) {
        e.preventDefault()
        void commit()
      }
    }}
  ></textarea>
  <div class="row">
    <label use:tooltip={'Sostituisce l’ultimo commit con questo (non farlo se è già stato pubblicato)'}>
      <input type="checkbox" bind:checked={amend} /> Modifica ultimo commit
    </label>
    <button class="go" disabled={reason !== ''} use:tooltip={reason || { text: 'Crea il commit con i file in stage', shortcut: '⌘↵' }} onclick={() => void commit()}>Commit</button>
  </div>
</div>

<style>
  .box { display: flex; flex-direction: column; gap: 6px; }
  textarea { resize: vertical; min-height: 40px; padding: 6px 8px; border-radius: 8px; background: var(--well); border: 1px solid var(--edge-soft); color: var(--txt); font: inherit; font-size: 12px; outline: 0; }
  textarea:focus { border-color: var(--accent); }
  textarea::placeholder { color: var(--faint); }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  label { display: flex; align-items: center; gap: 5px; font-size: 11.5px; color: var(--dim); cursor: pointer; }
  .go { padding: 4px 12px; border-radius: 8px; border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--txt); font: inherit; font-size: 12px; cursor: pointer; }
  .go:hover:not(:disabled) { background: var(--accent-line); }
  .go:disabled { opacity: 0.45; cursor: not-allowed; }
  @media (prefers-reduced-motion: no-preference) { .go, textarea { transition: border-color var(--dur), background var(--dur); } }
</style>
