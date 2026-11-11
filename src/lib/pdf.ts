// Caricato con import() dinamico alla prima apertura di un PDF: pdf.js resta fuori dal chunk iniziale.
// Il worker è un file locale del bundle (Vite `?worker`), nessun CDN.
import * as pdfjs from 'pdfjs-dist'
import PdfWorker from 'pdfjs-dist/build/pdf.worker.min.mjs?worker'

let worker: Worker | undefined

/** Apre un PDF dai byte. Niente script, niente eval, niente richieste di rete (né dati, né font, né CMap esterni). */
export function openPdf(data: Uint8Array) {
  if (!worker) {
    worker = new PdfWorker()
    pdfjs.GlobalWorkerOptions.workerPort = worker
  }
  // Niente scripting, XFA né annotation layer; nessun URL esterno (CMap, font, wasm): la CSP (senza unsafe-eval,
  // con connect-src solo IPC) è la garanzia che niente venga valutato o scaricato.
  return pdfjs.getDocument({
    data,
    disableAutoFetch: true,
    disableStream: true,
    enableXfa: false,
    useSystemFonts: false,
    verbosity: 0,
  })
}

export type { PDFDocumentProxy } from 'pdfjs-dist'
