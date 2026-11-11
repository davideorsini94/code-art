/**
 * Resta agganciato in fondo? `idChanged`: è cambiata la conversazione mostrata (si riparte dal fondo);
 * `scrolledUp`/`atBottom`: l'ultimo scroll dell'utente. Senza novità vale la scelta precedente.
 */
export function shouldStick(prev: boolean, o: { idChanged: boolean; scrolledUp?: boolean; atBottom?: boolean }): boolean {
  if (o.idChanged) return true
  if (o.scrolledUp) return false
  if (o.atBottom) return true
  return prev
}
