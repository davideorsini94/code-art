export type PaletteMode = 'files' | 'commands' | 'all' | 'modes'

class PaletteState {
  open = $state(false)
  mode = $state<PaletteMode>('all')

  /** Apre in `mode`; se è già aperta nella stessa modalità la chiude (stesso tasto = interruttore). */
  show(mode: PaletteMode) {
    if (this.open && this.mode === mode) {
      this.open = false
      return
    }
    this.mode = mode
    this.open = true
  }

  close() {
    this.open = false
  }
}

export const palette = new PaletteState()
