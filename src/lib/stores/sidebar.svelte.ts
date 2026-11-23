export type SidebarViewId = 'files' | 'search' | 'git' | 'convs' | 'ssh' | 'db' | 'settings'

export const SIDEBAR_VIEWS: readonly SidebarViewId[] = ['files', 'search', 'git', 'convs', 'ssh', 'db', 'settings']
const key = (projectId: string) => `codeart.sidebar.${projectId}`

function isView(v: unknown): v is SidebarViewId {
  return v === 'files' || v === 'search' || v === 'git' || v === 'convs' || v === 'ssh' || v === 'db' || v === 'settings'
}

/** Vista corrente del pannello laterale, ricordata per progetto. */
export class SidebarStore {
  view = $state<SidebarViewId>('files')
  private projectId: string | null = null

  /** Associa lo store al progetto della finestra e ne ripristina la vista (default: Esplora). */
  bind(projectId: string) {
    this.projectId = projectId
    let saved: unknown = null
    try {
      saved = localStorage.getItem(key(projectId))
    } catch {
      /* storage non disponibile */
    }
    this.view = isView(saved) ? saved : 'files'
  }

  setView(v: SidebarViewId) {
    this.view = v
    if (!this.projectId) return
    try {
      localStorage.setItem(key(this.projectId), v)
    } catch {
      /* la vista resta solo in memoria */
    }
  }
}

export const sidebar = new SidebarStore()
