import { invoke as tauriInvoke } from '@tauri-apps/api/core'

/**
 * Outside the Sangraha window (a plain browser on `npm run dev`), answers come from
 * `app/demo/board.json`: made-up repositories, for screenshots and layout work.
 */
const inTauri = '__TAURI_INTERNALS__' in window

async function demo<T>(cmd: string): Promise<T> {
  if (cmd === 'board' || cmd === 'refresh' || cmd.endsWith('_board') || cmd.endsWith('_refresh')) {
    const file = cmd.startsWith('apps')
      ? 'apps'
      : cmd.startsWith('security')
        ? 'security'
        : cmd.startsWith('releases')
          ? 'releases'
          : cmd.startsWith('tidy')
            ? 'tidy'
            : 'board'
    const b = (await (await fetch(`/demo/${file}.json`)).json()) as { checkedAt: number | null }
    b.checkedAt = Math.floor(Date.now() / 1000) - 180
    return b as T
  }
  if (cmd === 'set_hook' || cmd === 'subtree_pull') return 'Demo: nothing was changed' as T
  if (cmd === 'delete_merged') return ['Demo: nothing was changed'] as T
  if (cmd === 'save_digest_settings') return 'Demo: nothing was changed' as T
  if (cmd === 'digest_state') return { scheduled: false, cli: '/home/you/.cargo/bin/sangraha' } as T
  if (cmd === 'digest_preview')
    return {
      at: Math.floor(Date.now() / 1000),
      summary: '6 repos need attention · 2 app updates · 1 ready to tag · 4 merged branches to delete',
      items: [
        { id: 'a', area: 'Repos', text: 'harbor-notes: Checks failing on main', new: true },
        { id: 'b', area: 'Apps', text: 'Lantern Budget: 0.4.0 → v0.4.1', new: false },
      ],
      new: 1,
    } as T
  if (cmd === 'changelog')
    return '## 0.4.0\n\n### Added\n\n- Export to CSV (a1b2c3d)\n- Dark theme follows the system (b2c3d4e)\n\n### Fixed\n\n- Crash when the list is empty (#41) (c3d4e5f)\n' as T
  if (cmd === 'release_details')
    return {
      releaseUrl: 'https://github.com/you/harbor-notes/releases/tag/v0.3.2',
      prerelease: false,
      assets: [
        { name: 'harbor-notes-0.3.2-linux-x64.tar.gz', size: 9_437_184, downloads: 41 },
        { name: 'harbor-notes-0.3.2-windows-x64.zip', size: 8_912_896, downloads: 63 },
        { name: 'SHA256SUMS.txt', size: 212, downloads: 12 },
      ],
      runs: [{ name: 'Release', status: 'completed', conclusion: 'success', url: 'https://github.com/you/harbor-notes/actions' }],
    } as T
  if (cmd === 'bulk_plan' || cmd === 'bulk_apply') {
    const p = (await (await fetch('/demo/bulk.json')).json()) as BulkPlan
    if (cmd === 'bulk_apply') p.applied = true
    return p as T
  }
  if (cmd.startsWith('set') || cmd.startsWith('save') || cmd === 'settings')
    return {
      roots: ['/home/you/Work'],
      hidden: [],
      appsDir: null,
      extraApps: ['someone/their-app'],
      hiddenApps: [],
      prereleases: true,
      delayDays: 0,
      assetRules: {},
      termsFile: null,
      allowedEmails: [],
      requireSigned: false,
      digestCommand: null,
      digestTime: '09:00',
      runInTray: false,
    } as T
  if (cmd === 'version') return 'demo' as T
  return undefined as T
}

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return inTauri ? tauriInvoke<T>(cmd, args) : demo<T>(cmd)
}

export type Level = 'info' | 'low' | 'medium' | 'high'
export type Ci = 'passing' | 'failing' | 'running' | 'none'
export type ProblemKind =
  | 'ciFailing'
  | 'alerts'
  | 'unpushed'
  | 'uncommitted'
  | 'unreadable'
  | 'localOnly'
  | 'behind'
  | 'untracked'
  | 'stashes'
  | 'detached'
  | 'oldRemoteName'
  | 'notCloned'

export interface Problem {
  kind: ProblemKind
  level: Level
  text: string
  path: string | null
}

export type Action =
  | { kind: 'openUrl'; label: string; url: string }
  | { kind: 'openTerminal'; label: string; path: string }
  | { kind: 'openFolder'; label: string; path: string }

export interface Release {
  tag: string
  prerelease: boolean
  draft: boolean
  publishedAt: string | null
  url: string
}

export interface Alerts {
  total: number
  critical: number
  high: number
  moderate: number
  low: number
}

export interface LocalClone {
  path: string
  github: string | null
  otherRemote: string | null
  branch: string | null
  upstream: string | null
  ahead: number
  behind: number
  unpushed: number
  changed: number
  untracked: number
  stashes: number
  worktree: boolean
  error: string | null
}

export interface Row {
  key: string
  name: string
  fullName: string | null
  url: string | null
  description: string | null
  private: boolean
  archived: boolean
  fork: boolean
  language: string | null
  pushedAt: string | null
  defaultBranch: string | null
  ci: Ci
  release: Release | null
  alerts: Alerts | null
  openIssues: number
  openPrs: number
  foreign: boolean
  clones: LocalClone[]
  problems: Problem[]
  action: Action | null
  score: number
  hidden: boolean
}

export interface Counts {
  all: number
  attention: number
  unpushed: number
  ciRed: number
  alerts: number
  notCloned: number
  localOnly: number
  hidden: number
}

export interface Rate {
  limit: number
  remaining: number
  resetAt: string
}

export interface Board {
  login: string
  checkedAt: number | null
  rate: Rate | null
  platform: string
  roots: string[]
  rows: Row[]
  counts: Counts
  warnings: string[]
}

export interface Config {
  roots: string[]
  hidden: string[]
  appsDir: string | null
  extraApps: string[]
  hiddenApps: string[]
  prereleases: boolean
  delayDays: number
  assetRules: Record<string, string>
  termsFile: string | null
  allowedEmails: string[]
  requireSigned: boolean
  digestCommand: string | null
  digestTime: string
  runInTray: boolean
}

export interface Branch {
  name: string
  oid: string
  lastCommit: string | null
  pullRequest: number | null
}

export interface TidyRepo {
  repo: string
  private: boolean
  branches: number
  merged: Branch[]
  stale: Branch[]
  missing: string[]
  oldRuns: number | null
  largeFiles: { path: string; size: number }[]
}

export interface TidyBoard {
  repos: TidyRepo[]
  counts: { merged: number; stale: number; noReadme: number; noLicense: number; noCi: number; oldRuns: number; largeFiles: number }
  checkedAt: number | null
  warnings: string[]
}

export interface Digest {
  at: number
  summary: string
  items: { id: string; area: string; text: string; new: boolean }[]
  new: number
}

export type Setting =
  | 'securityAlerts'
  | 'securityFixes'
  | 'secretScanning'
  | 'pushProtection'
  | 'deleteBranchOnMerge'
  | 'autoMerge'
  | 'squashMerge'
  | 'mergeCommit'
  | 'rebaseMerge'

export type FileSource = { kind: 'dependabot' } | { kind: 'copy'; repo: string; path: string } | { kind: 'text'; text: string }

export interface BulkRequest {
  repos: string[]
  settings: Partial<Record<Setting, boolean>>
  addTopics: string[]
  removeTopics: string[]
  files: { path: string; source: FileSource; replace: boolean }[]
  pullRequest: boolean
  message: string
}

export interface BulkStep {
  label: string
  now: string
  then: string
  change: boolean
  note: string | null
}

export interface BulkPlan {
  repos: { repo: string; steps: BulkStep[]; error: string | null; done: string[]; pullRequest: string | null }[]
  changes: number
  author: string
  applied: boolean
}

export type Verdict = 'disagree' | 'needsBump' | 'ready' | 'noTag' | 'noVersion' | 'noChanges'

export interface ReleaseRow {
  repo: string
  path: string
  branch: string | null
  tag: string | null
  tagDate: string | null
  tagSha: string | null
  since: number
  pullRequests: number
  versions: { path: string; version: string }[]
  version: string | null
  verdict: Verdict
  note: string
  release: Release | null
  unreleasedTag: boolean
}

export interface Subtree {
  host: string
  path: string
  dir: string
  pulled: string
  pulledAt: string | null
  squash: boolean
  source: string | null
  branch: string
  behind: number | null
  state: 'current' | 'behind' | 'unknown'
  note: string | null
  edits: string[]
}

export interface ReleaseBoard {
  rows: ReleaseRow[]
  subtrees: Subtree[]
  counts: { ready: number; needsBump: number; disagree: number; unreleased: number; drift: number }
  checkedAt: number | null
  warnings: string[]
}

export interface ReleaseDetails {
  releaseUrl: string | null
  prerelease: boolean
  assets: { name: string; size: number; downloads: number }[]
  runs: { name: string; status: string; conclusion: string | null; url: string }[]
}

export type Severity = 'low' | 'moderate' | 'high' | 'critical'
export type Scope = 'runtime' | 'development' | 'unknown'

export interface SecurityAlert {
  repo: string
  number: number
  package: string
  ecosystem: string
  severity: Severity
  scope: Scope
  manifest: string | null
  requirement: string | null
  patched: string | null
  advisory: string
  summary: string
  url: string
}

export interface PackageGroup {
  ecosystem: string
  package: string
  severity: Severity
  shipped: boolean
  repos: string[]
  fix: string | null
  unfixed: number
  alerts: SecurityAlert[]
}

export interface RepoScan {
  repo: string
  secrets: number | null
  code: number | null
  codeSerious: number
}

export type HookState = 'none' | 'sangraha' | 'other' | 'shared'

export interface HookRow {
  path: string
  github: string | null
  state: HookState
}

export interface SecurityBoard {
  checkedAt: number | null
  rate: { limit: number; remaining: number; resetAt: string } | null
  groups: PackageGroup[]
  scans: RepoScan[]
  counts: {
    critical: number
    high: number
    moderate: number
    low: number
    shipped: number
    development: number
    secrets: number
    code: number
    secretsOff: number
    codeOff: number
  }
  hooks: HookRow[]
  cli: string | null
  warnings: string[]
}

export type AppStatus = 'update' | 'unknown' | 'waiting' | 'upToDate' | 'notInstalled' | 'noBuild'
export type Kind = 'archive' | 'appImage' | 'exe' | 'setup' | 'systemPackage' | 'apk' | 'mac'

export interface Offer {
  tag: string
  prerelease: boolean
  publishedAt: string | null
  url: string
  asset: string
  size: number
  kind: Kind
  checksum: boolean
}

export interface Here {
  version: string | null
  path: string
  launcher: string | null
  bySangraha: boolean
  previous: string | null
}

export interface AppRow {
  key: string
  name: string
  repo: string
  description: string | null
  url: string
  own: boolean
  status: AppStatus
  offer: Offer | null
  newest: string | null
  here: Here | null
  android: string | null
  notes: string[]
  hidden: boolean
}

export interface AppCounts {
  all: number
  installed: number
  updates: number
  notInstalled: number
  hidden: number
}

export interface AppsBoard {
  checkedAt: number | null
  rate: Rate | null
  appsDir: string
  rows: AppRow[]
  counts: AppCounts
  warnings: string[]
}

export interface InstallFailure {
  unverified: boolean
  message: string
}

export interface InstallProgress {
  key: string
  done: number
  total: number
}

export const api = {
  board: () => invoke<Board>('board'),
  refresh: (fetch: boolean) => invoke<Board>('refresh', { fetch }),
  settings: () => invoke<Config>('settings'),
  setRoots: (roots: string[]) => invoke<Config>('set_roots', { roots }),
  setHidden: (key: string, hidden: boolean) => invoke<Config>('set_hidden', { key, hidden }),
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  openFolder: (path: string) => invoke<void>('open_folder', { path }),
  openTerminal: (path: string) => invoke<void>('open_terminal', { path }),
  version: () => invoke<string>('version'),
  appsBoard: () => invoke<AppsBoard>('apps_board'),
  appsRefresh: () => invoke<AppsBoard>('apps_refresh'),
  installApp: (key: string, allowUnverified: boolean) => invoke<string>('install_app', { key, allowUnverified }),
  rollbackApp: (key: string) => invoke<string>('rollback_app', { key }),
  launchApp: (key: string) => invoke<void>('launch_app', { key }),
  setAppHidden: (key: string, hidden: boolean) => invoke<Config>('set_app_hidden', { key, hidden }),
  saveAppsSettings: (s: { appsDir: string; prereleases: boolean; delayDays: number; extraApps: string[]; rules: string[] }) =>
    invoke<Config>('save_apps_settings', s),
  securityBoard: () => invoke<SecurityBoard>('security_board'),
  securityRefresh: () => invoke<SecurityBoard>('security_refresh'),
  setHook: (path: string, on: boolean) => invoke<string>('set_hook', { path, on }),
  saveLeakSettings: (s: { termsFile: string; allowedEmails: string[]; requireSigned: boolean }) =>
    invoke<Config>('save_leak_settings', s),
  releasesBoard: () => invoke<ReleaseBoard>('releases_board'),
  releasesRefresh: () => invoke<ReleaseBoard>('releases_refresh'),
  releaseDetails: (repo: string, tag: string, sha: string) => invoke<ReleaseDetails>('release_details', { repo, tag, sha }),
  changelog: (path: string) => invoke<string>('changelog', { path }),
  subtreePull: (path: string, dir: string) => invoke<string>('subtree_pull', { path, dir }),
  tidyBoard: () => invoke<TidyBoard>('tidy_board'),
  tidyRefresh: () => invoke<TidyBoard>('tidy_refresh'),
  deleteMerged: (repo: string, names: string[]) => invoke<string[]>('delete_merged', { repo, names }),
  digestPreview: () => invoke<Digest>('digest_preview'),
  digestState: () => invoke<{ scheduled: boolean; cli: string | null }>('digest_state'),
  saveDigestSettings: (s: { command: string; time: string; daily: boolean; runInTray: boolean }) => invoke<string>('save_digest_settings', s),
  bulkPlan: (req: BulkRequest) => invoke<BulkPlan>('bulk_plan', { req }),
  bulkApply: (req: BulkRequest) => invoke<BulkPlan>('bulk_apply', { req }),
}

/** The tray read GitHub in the background: the tabs should load again. */
export async function onChecked(f: () => void): Promise<() => void> {
  if (!inTauri) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen('checked', () => f())
}

/** The repository a bulk change is working on. Does nothing outside the Sangraha window. */
export async function onBulkProgress(f: (repo: string) => void): Promise<() => void> {
  if (!inTauri) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<string>('bulk-progress', (e) => f(e.payload))
}

/** Download progress while an app installs. Does nothing outside the Sangraha window. */
export async function onInstallProgress(f: (p: InstallProgress) => void): Promise<() => void> {
  if (!inTauri) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<InstallProgress>('install-progress', (e) => f(e.payload))
}

export function runAction(a: Action): Promise<void> {
  switch (a.kind) {
    case 'openUrl':
      return api.openUrl(a.url)
    case 'openTerminal':
      return api.openTerminal(a.path)
    case 'openFolder':
      return api.openFolder(a.path)
  }
}
