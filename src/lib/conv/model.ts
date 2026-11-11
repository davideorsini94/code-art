/**
 * Modello da mostrare. L'ultima scelta esplicita riuscita (`conv.model`, salvata dal motore) vale finché un `init`
 * successivo non riporta altro (è ciò che gira davvero); senza scelte vale l'`init`, poi `conv.model`.
 * `lastSetAt` / `initAt`: istanti (ms) della scelta riuscita e dell'ultimo `init`; null se non avvenuti.
 */
export function displayedModel(o: { convModel: string | null; initModel: string | null; lastSetAt: number | null; initAt: number | null }): string {
  const explicit = o.lastSetAt != null && (o.initAt == null || o.lastSetAt >= o.initAt)
  return (explicit ? (o.convModel ?? o.initModel) : (o.initModel ?? o.convModel)) ?? ''
}

/** Alias integrati del compositore, nell'ordine in cui compaiono, con l'etichetta mostrata. */
export const MODEL_ALIASES = ['fable', 'opus', 'sonnet', 'haiku'] as const
const LABELS: Record<string, string> = { fable: 'Fable', opus: 'Opus', sonnet: 'Sonnet', haiku: 'Haiku' }

/** Etichetta amichevole di un alias; gli altri id (riportati dal processo) restano come sono. */
export const modelLabel = (m: string): string => LABELS[m] ?? m

/** Valori del selettore: gli alias, più il modello in uso se non è uno di essi. */
export function modelOptions(current: string): string[] {
  return current && !(MODEL_ALIASES as readonly string[]).includes(current) ? [...MODEL_ALIASES, current] : [...MODEL_ALIASES]
}
