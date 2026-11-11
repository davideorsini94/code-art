// Politica per documenti markdown grandi: il render (marked + DOMPurify) è sincrono sul thread principale.
export const LARGE_DOC = 400 * 1024
export const MEDIUM_DOC = 100 * 1024

export type PreviewPolicy = { large: boolean; debounceMs: number }

/** `length`: lunghezza del testo (≈ byte). Grande: anteprima solo su richiesta; medio: attesa più lunga. */
export function previewPolicy(length: number): PreviewPolicy {
  if (length > LARGE_DOC) return { large: true, debounceMs: 0 }
  return { large: false, debounceMs: length > MEDIUM_DOC ? 600 : 150 }
}
