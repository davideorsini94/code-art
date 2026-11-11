import { invoke } from '@tauri-apps/api/core'
import type { Project } from './types'

export const projectCurrent = () => invoke<Project>('project_current')
export const projectOpenFolder = (path: string) => invoke<Project>('project_open_folder', { path })
export const projectPickAndOpen = () => invoke<void>('project_pick_and_open')
export const projectRename = (id: string, name: string) => invoke<Project>('project_rename', { id, name })
export const projectAddFolder = (id: string, path: string) => invoke<Project>('project_add_folder', { id, path })
export const projectRelocate = (id: string, old: string, newPath: string) =>
  invoke<Project>('project_relocate', { id, old, new: newPath })
export const projectMissingFolders = (id: string) => invoke<string[]>('project_missing_folders', { id })
export const projectBranch = (id: string) => invoke<string | null>('project_branch', { id })
export const settingsGet = (key: string) => invoke<string | null>('settings_get', { key })
export const settingsSet = (key: string, value: string) => invoke<void>('settings_set', { key, value })

// ---- file system ----
import type { DirEntry, FileContent, FsError } from './types'
export const fsList = (path: string) => invoke<DirEntry[]>('fs_list', { path })
export const fsCreate = (path: string, isDir: boolean) => invoke<void>('fs_create', { path, isDir })
export const fsRename = (from: string, to: string) => invoke<void>('fs_rename', { from, to })
export const fsTrash = (path: string) => invoke<void>('fs_trash', { path })
export const fsRead = (path: string) => invoke<FileContent>('fs_read', { path })
export const fsWrite = (path: string, text: string, expectedMtimeMs: number | null) =>
  invoke<number>('fs_write', { path, text, expectedMtimeMs })

/** Normalizza un errore di comando in `FsError`. */
export function asFsError(e: unknown): FsError {
  if (e && typeof e === 'object' && 'code' in e) return e as FsError
  return { code: 'io', message: String(e) }
}

// ---- terminale ----
export const ptySpawn = (cwd: string, cols: number, rows: number) => invoke<{ id: number; shell: string }>('pty_spawn', { cwd, cols, rows })
export const ptyWrite = (id: number, data: string) => invoke<void>('pty_write', { id, data })
export const ptyResize = (id: number, cols: number, rows: number) => invoke<void>('pty_resize', { id, cols, rows })
export const ptyKill = (id: number) => invoke<void>('pty_kill', { id })

// ---- conversazioni ----
import type { ChatItem, Conversation } from './types'
export const convList = (projectId: string) => invoke<Conversation[]>('conv_list', { projectId })
export const convHistory = (sessionId: string) => invoke<ChatItem[]>('conv_history', { sessionId })

// ---- motore conversazioni ----
import type { ClaudeCheck, ClaudeEvent, ConvStartOpts } from './types'
export const claudeCheck = () => invoke<ClaudeCheck>('claude_check')
export const convStart = (projectId: string, opts: ConvStartOpts = {}) => invoke<string>('conv_start', { projectId, opts })
export const convResume = (sessionId: string) => invoke<void>('conv_resume', { sessionId })
export const convSend = (sessionId: string, text: string, imagePaths: string[] = []) =>
  invoke<void>('conv_send', { sessionId, text, imagePaths })
/** `choice`: permesso `allow`/`deny`; piano `approve`/`reject`; domanda = etichetta scelta o testo libero. */
export const convAnswer = (sessionId: string, requestId: string, choice: string) =>
  invoke<void>('conv_answer', { sessionId, requestId, choice })
export const convInterrupt = (sessionId: string) => invoke<void>('conv_interrupt', { sessionId })
export const convSetModel = (sessionId: string, model: string) => invoke<void>('conv_set_model', { sessionId, model })
export const convSetPermissionMode = (sessionId: string, mode: string) =>
  invoke<void>('conv_set_permission_mode', { sessionId, mode })
export const convMarkViewed = (sessionId: string) => invoke<void>('conv_mark_viewed', { sessionId })
export const convArchive = (sessionId: string, archived: boolean) => invoke<void>('conv_archive', { sessionId, archived })
export const convRunning = (projectId: string) => invoke<string[]>('conv_running', { projectId })
/** Ferma il processo della conversazione (interrompe un turno in corso); si riprende al prossimo invio. */
export const convStop = (sessionId: string) => invoke<void>('conv_stop', { sessionId })
/** Richieste (permesso/domanda/piano) ancora aperte, dalla più vecchia: servono a chi apre la scheda dopo che sono state emesse. */
export const convPending = (sessionId: string) => invoke<ClaudeEvent[]>('conv_pending', { sessionId })
export const convRename = (sessionId: string, title: string) => invoke<void>('conv_rename', { sessionId, title })
export const convUnlockTitle = (sessionId: string) => invoke<void>('conv_unlock_title', { sessionId })
/** Salva un'immagine incollata/trascinata nella cartella temporanea dell'app (validata dal motore); ritorna il percorso per `convSend`. */
export const convSaveImage = (bytes: number[], ext: string) => invoke<string>('conv_save_image', { bytes, ext })
