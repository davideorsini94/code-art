/** Errore dei comandi che eseguono codice (Claude, terminale) in un progetto non fidato. */
export const UNTRUSTED = 'untrusted_folder'
/** Testo leggibile di un errore di comando. */
export const errorText = (e: unknown) =>
  String(e) === UNTRUSTED ? 'Cartella non fidata: sola lettura, Claude e terminale disattivati' : String(e)
