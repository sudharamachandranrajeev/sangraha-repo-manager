import type { Level, ProblemKind, Row } from './api'

export type Tone = 'bad' | 'warn' | 'info' | 'ok' | 'muted'

/** One chip per row: the worst problem in a word or two. */
export function chip(row: Row): { label: string; tone: Tone } {
  if (row.archived) return { label: 'Archived', tone: 'muted' }
  const worst = row.problems[0]
  if (!worst) return { label: row.clones.length ? 'Clean' : 'On GitHub', tone: 'ok' }
  return { label: kindLabel(worst.kind, row), tone: toneOf(worst.level, worst.kind) }
}

export function kindLabel(kind: ProblemKind, row?: Row): string {
  switch (kind) {
    case 'ciFailing':
      return 'Checks failing'
    case 'alerts': {
      const n = row?.alerts?.total ?? 0
      return n === 1 ? '1 alert' : `${n} alerts`
    }
    case 'unpushed':
      return 'Unpushed'
    case 'uncommitted':
      return 'Uncommitted'
    case 'unreadable':
      return 'Unreadable'
    case 'localOnly':
      return 'Local only'
    case 'behind':
      return 'Behind'
    case 'untracked':
      return 'New files'
    case 'stashes':
      return 'Stashed'
    case 'detached':
      return 'Detached'
    case 'oldRemoteName':
      return 'Old remote name'
    case 'notCloned':
      return 'Not cloned'
  }
}

export function toneOf(level: Level, kind?: ProblemKind): Tone {
  if (kind === 'notCloned') return 'info'
  switch (level) {
    case 'high':
      return 'bad'
    case 'medium':
      return 'warn'
    case 'low':
      return 'muted'
    case 'info':
      return 'info'
  }
}

export function ago(iso: string | null | number): string {
  if (iso === null) return '—'
  const t = typeof iso === 'number' ? iso * 1000 : Date.parse(iso)
  if (Number.isNaN(t)) return '—'
  const s = Math.max(0, (Date.now() - t) / 1000)
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.floor(s / 60)} min ago`
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`
  const d = Math.floor(s / 86400)
  if (d < 45) return d === 1 ? 'yesterday' : `${d} days ago`
  return new Date(t).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}

export function shortDate(iso: string | null): string {
  if (!iso) return ''
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return ''
  return new Date(t).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}

/** "~/Work/setu" for a path under the home folder. */
export function tidyPath(p: string, roots: string[]): string {
  for (const r of roots) {
    const parent = r.replace(/[\\/][^\\/]+[\\/]?$/, '')
    if (parent && p.startsWith(parent)) {
      const rest = p.slice(parent.length).replace(/^[\\/]/, '')
      return rest
    }
  }
  return p
}

export function folderName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p
}

export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`
}

/** A download size: "212 B", "38 KB", "9 MB", "1.2 GB". */
export function size(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} KB`
  return `${bytes} B`
}
