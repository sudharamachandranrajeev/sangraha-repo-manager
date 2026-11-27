<script lang="ts">
  import { onMount } from 'svelte'
  import { api, onInstallProgress, type AppRow, type AppsBoard, type InstallFailure } from './api'
  import { ago, plural, shortDate, size } from './format'
  import Menu, { type MenuItem } from './Menu.svelte'

  let {
    view,
    active,
    busy = $bindable(''),
  }: { view: 'table' | 'grid'; active: boolean; busy?: '' | 'refresh' } = $props()

  type Filter = 'all' | 'installed' | 'updates' | 'notInstalled' | 'hidden'

  let board = $state<AppsBoard | null>(null)
  let filter = $state<Filter>('all')
  let query = $state('')
  let error = $state('')
  let note = $state('')
  let search: HTMLInputElement
  let tick = $state(0)
  /** Download progress (0–100) of the app installing now, by key. */
  let progress = $state<Record<string, number>>({})
  let working = $state<string | null>(null)
  /** An app whose release has no SHA-256: install only after the owner says so. */
  let unverified = $state<{ row: AppRow; message: string } | null>(null)
  let preview = $state<AppRow[] | null>(null)
  let showWarnings = $state(false)

  const filters: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'installed', label: 'Installed' },
    { id: 'updates', label: 'Updates' },
    { id: 'notInstalled', label: 'Not installed' },
  ]

  const label: Record<AppRow['status'], { text: string; tone: string }> = {
    update: { text: 'Update', tone: 'warn' },
    unknown: { text: 'Version unknown', tone: 'info' },
    waiting: { text: 'Waiting', tone: 'muted' },
    upToDate: { text: 'Up to date', tone: 'ok' },
    notInstalled: { text: 'Not installed', tone: 'muted' },
    noBuild: { text: 'No download here', tone: 'muted' },
  }

  function matches(r: AppRow, f: Filter): boolean {
    if (f === 'hidden') return r.hidden
    if (r.hidden) return false
    switch (f) {
      case 'all':
        return true
      case 'installed':
        return !!r.here
      case 'updates':
        return r.status === 'update'
      case 'notInstalled':
        return r.status === 'notInstalled'
    }
  }

  const rows = $derived.by(() => {
    if (!board) return []
    const q = query.trim().toLowerCase()
    return board.rows.filter(
      (r) =>
        matches(r, filter) &&
        (!q || r.name.toLowerCase().includes(q) || r.key.toLowerCase().includes(q) || (r.description ?? '').toLowerCase().includes(q)),
    )
  })

  const updates = $derived(board ? board.rows.filter((r) => !r.hidden && r.status === 'update') : [])

  function count(f: Filter): number {
    if (!board) return 0
    const c = board.counts
    return { all: c.all, installed: c.installed, updates: c.updates, notInstalled: c.notInstalled, hidden: c.hidden }[f]
  }

  export function focusSearch() {
    search?.focus()
  }

  export async function load() {
    try {
      board = await api.appsBoard()
    } catch (e) {
      error = String(e)
    }
  }

  export async function refresh() {
    if (busy) return
    busy = 'refresh'
    error = ''
    try {
      board = await api.appsRefresh()
    } catch (e) {
      error = String(e)
    } finally {
      busy = ''
    }
  }


  /** Installs one app. Returns false when it did not install. */
  async function install(row: AppRow, allowUnverified = false): Promise<boolean> {
    working = row.key
    progress[row.key] = 0
    error = ''
    try {
      note = await api.installApp(row.key, allowUnverified)
      return true
    } catch (e) {
      const f = e as InstallFailure
      if (f && typeof f === 'object' && f.unverified) unverified = { row, message: f.message }
      else error = `${row.name}: ${f?.message ?? String(e)}`
      return false
    } finally {
      delete progress[row.key]
      working = null
      await load()
    }
  }

  async function updateAll(list: AppRow[]) {
    preview = null
    const done: string[] = []
    const failed: string[] = []
    for (const r of list) {
      if (await install(r)) done.push(r.name)
      else if (error) failed.push(error)
    }
    note = done.length ? `Updated ${done.join(', ')}.` : ''
    error = failed.join(' · ')
  }

  async function primary(row: AppRow) {
    try {
      switch (row.status) {
        case 'update':
        case 'notInstalled':
        case 'unknown':
          await install(row)
          break
        case 'upToDate':
          await api.launchApp(row.key)
          break
        default:
          await api.openUrl(row.offer?.url ?? `${row.url}/releases`)
      }
    } catch (e) {
      error = String(e)
    }
  }

  function primaryLabel(r: AppRow): string {
    switch (r.status) {
      case 'update':
        return 'Update'
      case 'notInstalled':
        return 'Install'
      case 'unknown':
        return 'Install latest'
      case 'upToDate':
        return r.here?.launcher ? 'Open' : 'Release notes'
      default:
        return 'Releases'
    }
  }

  function items(r: AppRow): MenuItem[] {
    const list: MenuItem[] = []
    if (r.offer) list.push({ label: `Release notes for ${r.offer.tag}`, run: () => api.openUrl(r.offer!.url) })
    list.push({ label: 'Open on GitHub', run: () => api.openUrl(r.url) })
    if (r.here) {
      const h = r.here
      const dir = h.path.endsWith('.AppImage') ? h.path.replace(/[/\\][^/\\]*$/, '') : h.path
      list.push({ label: 'Open install folder', run: () => api.openFolder(dir) })
      list.push({ label: 'Copy install path', run: () => navigator.clipboard.writeText(h.path) })
      if (h.bySangraha && h.previous) {
        list.push({
          label: `Roll back to ${h.previous}`,
          run: async () => {
            note = await api.rollbackApp(r.key)
            await load()
          },
        })
      }
      if (r.status === 'upToDate' && r.offer) list.push({ label: 'Install again', run: () => install(r) })
    }
    list.push({
      label: r.hidden ? 'Show in the list' : 'Hide from the list',
      run: async () => {
        await api.setAppHidden(r.key, !r.hidden)
        await load()
      },
    })
    return list
  }

  function facts(r: AppRow): string[] {
    const out: string[] = []
    if (r.offer) out.push(size(r.offer.size), shortDate(r.offer.publishedAt))
    if (r.android) out.push(`Android ${r.android}`)
    return out
  }

  function version(r: AppRow): string {
    if (!r.offer) return r.here ? `${r.here.version ?? 'installed'} here` : r.newest ? `${r.newest} on GitHub` : ''
    const here = r.here ? (r.here.version ?? 'unknown version') : 'not installed'
    const there = r.offer ? r.offer.tag : r.newest ?? '—'
    return r.status === 'upToDate' ? there : `${here} → ${there}`
  }

  // GitHub is read when the tab is first opened (and the last answer is old), not at start.
  let checked = false
  $effect(() => {
    if (!active || checked || !board) return
    checked = true
    if (!board.checkedAt || Date.now() / 1000 - board.checkedAt > 600) refresh()
  })

  onMount(() => {
    let stop = () => {}
    onInstallProgress((p) => {
      if (p.total) progress[p.key] = Math.min(100, Math.round((p.done * 100) / p.total))
    }).then((s) => (stop = s))
    load()
    const t = setInterval(() => tick++, 30_000)
    return () => {
      stop()
      clearInterval(t)
    }
  })
</script>

<div class="bar">
  <div class="filters caps" role="tablist" aria-label="Filter apps">
    {#each filters as f (f.id)}
      <button role="tab" aria-selected={filter === f.id} class:on={filter === f.id} onclick={() => (filter = f.id)}>
        {f.label} <span class="n" class:hot={f.id === 'updates' && count(f.id) > 0}>{count(f.id)}</span>
      </button>
    {/each}
    {#if board && board.counts.hidden}
      <button role="tab" aria-selected={filter === 'hidden'} class:on={filter === 'hidden'} onclick={() => (filter = 'hidden')}>
        Hidden <span class="n">{board.counts.hidden}</span>
      </button>
    {/if}
  </div>
  <div class="spacer"></div>
  <button class="btn caps" disabled={!updates.length || !!working} onclick={() => (preview = updates)}>Update all ({updates.length})</button>
  <input bind:this={search} type="search" placeholder="Search apps  /" bind:value={query} aria-label="Search apps" />
</div>

{#if error}
  <div class="banner bad" role="alert">
    <span>{error}</span>
    <button class="btn ghost" onclick={() => (error = '')}>Dismiss</button>
  </div>
{/if}
{#if note}
  <div class="banner" role="status">
    <span>{note}</span>
    <button class="btn ghost" onclick={() => (note = '')}>OK</button>
  </div>
{/if}

<main>
  {#if !board}
    <p class="empty">Looking at installed apps…</p>
  {:else if !rows.length}
    <p class="empty">
      {query ? 'No app matches.' : filter === 'updates' ? 'Every installed app is up to date.' : 'Nothing here.'}
      {#if !board.checkedAt}<br />Releases have not been read yet: press R or the refresh button.{/if}
    </p>
  {:else if view === 'grid'}
    <div class="grid">
      {#each rows as r (r.key)}
        {@const l = label[r.status]}
        <article class="card">
          <div class="cardtop caps">
            <span class="chip tone-{l.tone}">{l.text}</span>
            {#if r.offer?.prerelease}<span class="tag">pre-release</span>{/if}
            <span class="spacer"></span>
            {#if !r.own}<span class="tag">followed</span>{/if}
          </div>
          <h3>{r.name}</h3>
          <p class="mono small ver">{version(r)}</p>
          <p class="desc clamp">{r.description ?? r.repo}</p>
          <p class="facts small">
            {facts(r).join(' · ')}{#if r.offer && !r.offer.checksum}{facts(r).length ? ' · ' : ''}<span class="warn">no checksum</span>{/if}
          </p>
          {#each r.notes as n (n)}<p class="first small">{n}</p>{/each}
          <div class="cardact">
            {#if working === r.key}
              <div class="meter" role="progressbar" aria-label="Installing {r.name}" aria-valuenow={progress[r.key] ?? 0} aria-valuemin="0" aria-valuemax="100">
                <span style="width: {progress[r.key] ?? 0}%"></span><em class="caps">Installing {progress[r.key] ?? 0}%</em>
              </div>
            {:else}
              <button class="btn caps primary" class:quiet={r.status !== 'update' && r.status !== 'notInstalled'} disabled={!!working || r.status === 'waiting'} onclick={() => primary(r)}>{primaryLabel(r)}</button>
            {/if}
            <Menu label="More actions for {r.name}" items={items(r)} onerror={(m) => (error = m)} />
          </div>
        </article>
      {/each}
    </div>
  {:else}
    <table class="board">
      <thead class="caps">
        <tr>
          <th class="name">App</th>
          <th>Status</th>
          <th>Here</th>
          <th>On GitHub</th>
          <th>Download</th>
          <th>Android</th>
          <th class="act"><span class="sr">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        {#each rows as r (r.key)}
          {@const l = label[r.status]}
          <tr>
            <td class="name">
              <div class="title">{r.name}{#if !r.own}<span class="tag caps">followed</span>{/if}</div>
              <div class="desc">{r.notes[0] ?? r.description ?? r.repo}</div>
            </td>
            <td><span class="chip caps tone-{l.tone}">{l.text}</span></td>
            <td class="mono small">{r.here ? (r.here.version ?? '?') : '—'}</td>
            <td class="mono small">
              {#if r.offer}<span class:pre={r.offer.prerelease}>{r.offer.tag}</span>
                <div class="desc">{shortDate(r.offer.publishedAt)}</div>{:else}{r.newest ?? '—'}{/if}
            </td>
            <td class="small">
              {#if r.offer}{size(r.offer.size)}{#if !r.offer.checksum}{' · '}<span class="warn">no checksum</span>{/if}{:else}—{/if}
            </td>
            <td class="mono small">{r.android ?? '—'}</td>
            <td class="act">
              <div class="actcell">
                {#if working === r.key}
                  <div class="meter" role="progressbar" aria-label="Installing {r.name}" aria-valuenow={progress[r.key] ?? 0} aria-valuemin="0" aria-valuemax="100">
                    <span style="width: {progress[r.key] ?? 0}%"></span><em class="caps">{progress[r.key] ?? 0}%</em>
                  </div>
                {:else}
                  <button class="btn caps primary" class:quiet={r.status !== 'update' && r.status !== 'notInstalled'} disabled={!!working || r.status === 'waiting'} onclick={() => primary(r)}>{primaryLabel(r)}</button>
                {/if}
                <Menu label="More actions for {r.name}" items={items(r)} onerror={(m) => (error = m)} />
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</main>

<footer class="foot caps" data-tick={tick}>
  <span>Checked {board?.checkedAt ? ago(board.checkedAt) : 'never'}</span>
  {#if board?.rate}
    <span class:lowbudget={board.rate.remaining < 500}>GitHub budget {board.rate.remaining.toLocaleString()}/{board.rate.limit.toLocaleString()}</span>
  {/if}
  {#if board?.warnings.length}
    <button class="linkish caps" onclick={() => (showWarnings = !showWarnings)}>{plural(board.warnings.length, 'note', 'notes')}</button>
  {/if}
  <span class="spacer"></span>
  <span class="path" title="New apps are installed here">{board?.appsDir ?? ''}</span>
</footer>
{#if showWarnings && board?.warnings.length}
  <ul class="warnings small">
    {#each board.warnings as w, i (i)}<li>{w}</li>{/each}
  </ul>
{/if}

{#if preview}
  <div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (preview = null)}>
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="ua-title">
      <h2 id="ua-title" class="caps">Update {plural(preview.length, 'app', 'apps')}</h2>
      <p class="small">Each download is checked against its published SHA-256 first. An app that is running is skipped. The version it replaces is kept, so you can roll back.</p>
      <ul class="plan">
        {#each preview as r (r.key)}
          <li><span>{r.name}</span><span class="mono small">{r.here?.version ?? '?'} → {r.offer?.tag}</span><span class="small muted">{r.offer ? size(r.offer.size) : ''}</span></li>
        {/each}
      </ul>
      <p class="small muted">Download: {size(preview.reduce((n, r) => n + (r.offer?.size ?? 0), 0))} in all.</p>
      <div class="dlgact">
        <button class="btn ghost" onclick={() => (preview = null)}>Cancel</button>
        <button class="btn caps primary" onclick={() => updateAll(preview ?? [])}>Update {preview.length}</button>
      </div>
    </div>
  </div>
{/if}

{#if unverified}
  {@const u = unverified}
  <div class="scrim" role="presentation">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="uv-title" aria-describedby="uv-text">
      <h2 id="uv-title" class="caps">No checksum</h2>
      <p id="uv-text">{u.message}. Install {u.row.name} without checking that the file is the one its publisher uploaded?</p>
      <div class="dlgact">
        <button class="btn ghost" onclick={() => (unverified = null)}>Cancel</button>
        <button class="btn caps" onclick={() => ((unverified = null), install(u.row, true))}>Install anyway</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .path {
    text-transform: none;
    letter-spacing: 0;
  }
  .ver {
    margin: -4px 0 0;
    color: var(--muted);
  }
  .meter {
    position: relative;
    flex: 1;
    min-width: 132px;
    height: 34px;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    overflow: hidden;
  }
  .meter span {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--ink);
    opacity: 0.18;
    transition: width 200ms linear;
  }
  .meter em {
    position: relative;
    display: grid;
    place-items: center;
    height: 100%;
    font-style: normal;
  }
  .plan li {
    display: grid;
    padding: 8px 0;
    grid-template-columns: 1fr auto 70px;
    gap: 12px;
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
  }
  .plan li span:last-child {
    text-align: right;
  }
</style>
