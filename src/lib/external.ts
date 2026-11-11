import { openUrl } from '@tauri-apps/plugin-opener'
import { isSafeLink } from './safe-markdown'

/** Unica via per aprire un link esterno: solo http(s), nel browser di sistema; il resto è ignorato.
 *  Restituisce sempre `true` ("gestito"), così chi chiama (es. Monaco) non ripiega su `window.open`. */
export function openExternal(href: string): true {
  if (isSafeLink(href)) void openUrl(href.trim()).catch(() => {})
  return true
}
