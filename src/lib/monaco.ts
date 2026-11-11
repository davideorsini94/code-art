// Caricato con import() dinamico al primo file aperto: non finisce nel chunk iniziale.
import * as monaco from 'monaco-editor'
import { openExternal } from './external'
import { monacoColors } from './theme'
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker'
import JsonWorker from 'monaco-editor/language/json/json.worker.js?worker'
import CssWorker from 'monaco-editor/language/css/css.worker.js?worker'
import HtmlWorker from 'monaco-editor/language/html/html.worker.js?worker'
import TsWorker from 'monaco-editor/language/typescript/ts.worker.js?worker'

self.MonacoEnvironment = {
  getWorker(_id: string, label: string) {
    if (label === 'json') return new JsonWorker()
    if (label === 'css' || label === 'scss' || label === 'less') return new CssWorker()
    if (label === 'html' || label === 'handlebars' || label === 'razor') return new HtmlWorker()
    if (label === 'typescript' || label === 'javascript') return new TsWorker()
    return new EditorWorker()
  },
}

monaco.editor.defineTheme('codeart', {
  base: 'vs-dark',
  inherit: true,
  rules: [],
  colors: monacoColors,
})

// ⌘-clic su un link nel codice: mai `window.open`; solo http(s), nel browser di sistema. Il resto è ignorato
// (ma dichiarato gestito, così Monaco non prova altre vie).
monaco.editor.registerLinkOpener({ open: (uri) => openExternal(uri.toString(true)) })

export { monaco }
