import { fsReadBytes } from './ipc'
import { mimeFor } from './viewers'

/** Legge il file (confinato dal core) e ne fa un URL `blob:` con il tipo MIME della tabella fissa per estensione. */
export async function loadBlobUrl(path: string, name: string): Promise<{ url: string; size: number }> {
  const bytes = await fsReadBytes(path)
  const blob = new Blob([bytes as BlobPart], { type: mimeFor(name) ?? 'application/octet-stream' })
  return { url: URL.createObjectURL(blob), size: bytes.byteLength }
}

export function formatSize(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}
