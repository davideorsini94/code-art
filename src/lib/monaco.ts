// Caricato con import() dinamico al primo file aperto: non finisce nel chunk iniziale.
import * as monaco from 'monaco-editor'
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
  colors: {
    'editor.background': '#00000000', // lo sfondo lo dà .surface-opaque
    'editorGutter.background': '#00000000',
    'minimap.background': '#00000000',
    'editor.foreground': '#e9ecf5',
    'editorLineNumber.foreground': '#667089',
    'editorLineNumber.activeForeground': '#9aa3b8',
    'editor.lineHighlightBackground': '#ffffff0d',
    'editor.selectionBackground': '#8fa8ff40',
    'editorCursor.foreground': '#8fa8ff',
    'editorWidget.background': '#181b26',
    'scrollbarSlider.background': '#ffffff1a',
  },
})

export { monaco }
