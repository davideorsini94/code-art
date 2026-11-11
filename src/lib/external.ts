import { openUrl } from '@tauri-apps/plugin-opener'

/** Link che si possono aprire fuori dall'app: solo http(s). */
export function isSafeLink(href: string): boolean {
  return /^https?:/i.test(href.trim())
}

/** Unica via per aprire un link esterno: solo http(s), nel browser di sistema; il resto è ignorato.
 *  Restituisce sempre `true` ("gestito"), così chi chiama (es. Monaco) non ripiega su `window.open`. */
export function openExternal(href: string): true {
  if (isSafeLink(href)) void Promise.resolve(href.trim()).then(openUrl).catch(() => {})
  return true
}
