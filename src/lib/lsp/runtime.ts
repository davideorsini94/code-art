/** Collegamento di produzione del ponte LSP: core (ipc/eventi), progetto, editor e sanificatore markdown.
 *  Caricato con `import()` al primo file di un linguaggio con server. */
import type * as M from 'monaco-editor'
import { listenProject } from '../events'
import { lspSend, lspStart, lspStop } from '../ipc'
import { renderMarkdown } from '../safe-markdown'
import { editor } from '../stores/editor.svelte'
import { lspStore } from '../stores/lsp.svelte'
import { projectStore } from '../stores/project.svelte'
import { LANG_LABEL } from './languages'
import { createBridge, type LspApi, type LspBridge } from './monaco-bridge'

const api = (projectId: string): LspApi => ({
  start: (lang) => lspStart(lang),
  send: (id, message) => lspSend(id, message),
  stop: (id) => lspStop(id),
  onMessage: (cb) => listenProject<{ id: number; message: string }>('lsp:message', projectId, (e) => cb(e.payload.id, e.payload.message)),
  onExit: (cb) => listenProject<{ id: number; code: number }>('lsp:exit', projectId, (e) => cb(e.payload.id, e.payload.code)),
})

let bridge: LspBridge | null = null

function get(monaco: typeof M): LspBridge | null {
  if (bridge) return bridge
  const id = projectStore.project?.id
  if (!id) return null
  bridge = createBridge({
    monaco,
    api: api(id),
    folders: () => projectStore.project?.folders ?? [],
    openFile: (path, line, col) => void editor.openAt(path, line, col),
    render: renderMarkdown,
    onStatus: (lang, state, detail) => lspStore.setStatus(lang, state, detail),
    notify: (lang, text) => (lspStore.notice = `${LANG_LABEL[lang]}: ${text}`),
  })
  return bridge
}

export function attach(monaco: typeof M, model: M.editor.ITextModel, path: string) {
  get(monaco)?.attach(model, path)
}

export function didSave(model: M.editor.ITextModel) {
  bridge?.didSave(model)
}
