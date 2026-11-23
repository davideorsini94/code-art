<script lang="ts">
  import { onMount } from 'svelte'
  import { settingsGet, settingsSet } from '../lib/ipc'
  import { errorText } from '../lib/errors'
  import { PERMISSION_MODES } from '../lib/conv/labels'

  /** Valori usati dall'app quando l'impostazione non è salvata (come in engine.rs e git/remote.rs). */
  const DEFAULT_MODE = 'bypassPermissions'
  const DEFAULT_PULL = 'ff-only'

  const PULL_MODES: [mode: string, label: string, hint: string][] = [
    ['ff-only', 'Solo avanzamento rapido', 'si ferma se il ramo locale e quello remoto sono divergenti'],
    ['rebase', 'Rebase', 'riapplica i tuoi commit sopra quelli remoti'],
    ['merge', 'Merge', 'unisce i due rami con un commit di merge'],
  ]

  type Field = { key: string; value: string; saved: string }
  const mode = $state<Field>({ key: 'default_permission_mode', value: DEFAULT_MODE, saved: DEFAULT_MODE })
  const pull = $state<Field>({ key: 'git_pull_mode', value: DEFAULT_PULL, saved: DEFAULT_PULL })
  let error = $state<string | null>(null)

  async function load(f: Field, valid: string[]) {
    const v = await settingsGet(f.key).catch(() => null)
    if (v && valid.includes(v)) f.value = f.saved = v
  }

  onMount(() => {
    void load(mode, PERMISSION_MODES.map(([m]) => m))
    void load(pull, PULL_MODES.map(([m]) => m))
  })

  // scelta salvata subito; se il salvataggio fallisce torna la scelta precedente
  async function save(f: Field) {
    error = null
    try {
      await settingsSet(f.key, f.value)
      f.saved = f.value
    } catch (e) {
      f.value = f.saved
      error = errorText(e)
    }
  }
</script>

<div class="panel">
  <div class="head"><span class="title">Impostazioni</span></div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}

  <fieldset>
    <legend>Permessi di Claude</legend>
    <p class="note">Modalità con cui partono le nuove conversazioni. Puoi sempre cambiarla nella singola conversazione.</p>
    {#each PERMISSION_MODES as [m, label] (m)}
      <label class="opt"><input type="radio" name="perm" value={m} bind:group={mode.value} onchange={() => save(mode)} />{label}</label>
    {/each}
  </fieldset>

  <fieldset>
    <legend>Pull di Git</legend>
    <p class="note">Come «Pull» unisce le modifiche del ramo remoto.</p>
    {#each PULL_MODES as [m, label, hint] (m)}
      <div class="opt">
        <input type="radio" id="pull-{m}" name="pull" value={m} aria-describedby="pull-{m}-hint" bind:group={pull.value} onchange={() => save(pull)} />
        <div><label for="pull-{m}">{label}</label><span class="hint" id="pull-{m}-hint">{hint}</span></div>
      </div>
    {/each}
  </fieldset>
</div>

<style>
  .panel { height: 100%; overflow: auto; font-size: 12.5px; }
  .head { display: flex; align-items: center; padding: 8px 12px; }
  .title { font-weight: 650; }
  fieldset { border: 0; margin: 0; padding: 6px 12px 12px; display: flex; flex-direction: column; gap: 4px; }
  legend { padding: 0; font-weight: 600; color: var(--txt); }
  .note { margin: 2px 0 6px; color: var(--faint); font-size: 12px; line-height: 1.4; }
  .opt { display: flex; align-items: flex-start; gap: 8px; padding: 4px 6px; border-radius: 6px; color: var(--txt); cursor: pointer; }
  .opt:hover { background: var(--glass); }
  .opt label { cursor: pointer; }
  .opt input { margin: 2px 0 0; accent-color: var(--accent, currentColor); }
  .hint { display: block; color: var(--faint); font-size: 11.5px; margin-top: 1px; }
  .err { margin: 0 12px 6px; color: var(--err); font-size: 12px; overflow-wrap: anywhere; }
</style>
