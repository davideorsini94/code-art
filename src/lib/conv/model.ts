/**
 * Modello da mostrare. L'ultima scelta esplicita riuscita (`conv.model`, salvata dal motore) vale finché un `init`
 * successivo non riporta altro (è ciò che gira davvero); senza scelte vale l'`init`, poi `conv.model`.
 * `lastSetAt` / `initAt`: istanti (ms) della scelta riuscita e dell'ultimo `init`; null se non avvenuti.
 */
export function displayedModel(o: { convModel: string | null; initModel: string | null; lastSetAt: number | null; initAt: number | null }): string {
  const explicit = o.lastSetAt != null && (o.initAt == null || o.lastSetAt >= o.initAt)
  return (explicit ? (o.convModel ?? o.initModel) : (o.initModel ?? o.convModel)) ?? ''
}
