import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type { EventCallback, UnlistenFn } from '@tauri-apps/api/event'

/** Eventi del core legati a un progetto. Il nome reale porta l'id del progetto (`conv:event:<id>`): lo stesso
 *  schema di `projects::project_event` in src-tauri/src/projects.rs. */
export type ProjectEvent = 'conv:event' | 'conv:upsert' | 'conv:removed' | 'conv:focus' | 'fs:changed' | 'pty:data' | 'pty:exit'

export const projectEventName = (name: ProjectEvent, projectId: string) => `${name}:${projectId}`

/** Ascolta un evento del progetto sulla SOLA finestra corrente (mai `listen` globale: riceverebbe gli eventi
 *  inviati alle altre finestre). */
export function listenProject<T>(name: ProjectEvent, projectId: string, cb: EventCallback<T>): Promise<UnlistenFn> {
  return getCurrentWebviewWindow().listen<T>(projectEventName(name, projectId), cb)
}
