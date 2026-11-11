/** Un file o un link rilasciato fuori da una zona di rilascio (compositore, albero) farebbe navigare la WebView
 *  verso di lui: a livello di finestra `dragover` e `drop` vengono sempre annullati. Le zone gestiscono prima i
 *  propri eventi (bolla); qui, se nessuna l'ha fatto, si mostra anche "rilascio non consentito". */
export function guardWindowDrops(win: Window = window): () => void {
  const over = (e: DragEvent) => {
    if (!e.defaultPrevented && e.dataTransfer) e.dataTransfer.dropEffect = 'none'
    e.preventDefault()
  }
  const drop = (e: DragEvent) => e.preventDefault()
  win.addEventListener('dragover', over)
  win.addEventListener('drop', drop)
  return () => {
    win.removeEventListener('dragover', over)
    win.removeEventListener('drop', drop)
  }
}
