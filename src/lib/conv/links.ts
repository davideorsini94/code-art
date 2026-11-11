import { openUrl } from '@tauri-apps/plugin-opener'
import { isSafeLink } from '../safe-markdown'

/** Click su un link dentro markdown sanificato: mai navigare la WebView; solo http(s)/mailto, nel browser di sistema. */
function openLink(e: MouseEvent) {
  const a = (e.target as Element | null)?.closest?.('a')
  if (!a) return
  e.preventDefault()
  const href = a.getAttribute('href')
  if (href && isSafeLink(href)) void openUrl(href)
}

/** Azione Svelte: intercetta click e click centrale sui link del nodo. Da usare su ogni contenitore di `renderMarkdown`. */
export function links(node: HTMLElement) {
  node.addEventListener('click', openLink)
  node.addEventListener('auxclick', openLink)
  return {
    destroy() {
      node.removeEventListener('click', openLink)
      node.removeEventListener('auxclick', openLink)
    },
  }
}
