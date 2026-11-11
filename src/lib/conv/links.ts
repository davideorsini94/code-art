import { openUrl } from '@tauri-apps/plugin-opener'
import { isSafeLink } from '../safe-markdown'

/** Click su un link dentro markdown sanificato: mai navigare la WebView; solo http(s)/mailto, nel browser di sistema. */
export function openLink(e: MouseEvent) {
  const a = (e.target as Element | null)?.closest?.('a')
  if (!a) return
  e.preventDefault()
  const href = a.getAttribute('href')
  if (href && isSafeLink(href)) void openUrl(href)
}
