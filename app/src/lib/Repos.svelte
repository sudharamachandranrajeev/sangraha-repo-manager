<script lang="ts">
  import { onMount } from 'svelte'
  import { api, runAction, type Board, type Row } from './api'
  import { ago, chip, folderName, plural, shortDate } from './format'
  import Detail from './Detail.svelte'
  import RowMenu from './RowMenu.svelte'

  type Filter = 'all' | 'attention' | 'unpushed' | 'ciRed' | 'alerts' | 'notCloned' | 'localOnly' | 'hidden'
  type Sort = 'worst' | 'name' | 'pushed' | 'release'

  let { view, busy = $bindable('') }: { view: 'table' | 'grid'; busy?: '' | 'refresh' | 'fetch' } = $props()

  let board = $state<Board | null>(null)
  let filter = $state<Filter>('attention')
  let sort = $state<Sort>('worst')
  let query = $state('')
  let error = $state('')
  let note = $state('')
  let open = $state<string | null>(null)
  let showWarnings = $state(false)
  let search: HTMLInputElement
  let tick = $state(0)

  const filters: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'attention', label: 'Needs attention' },
    { id: 'unpushed', label: 'Unpushed' },
    { id: 'ciRed', label: 'Checks failing' },
    { id: 'alerts', label: 'Alerts' },
    { id: 'notCloned', label: 'Not cloned' },
    { id: 'localOnly', label: 'Local only' },
  ]

  function matches(r: Row, f: Filter): boolean {
    if (f === 'hidden') return r.hidden
    if (r.hidden) return false
    const has = (k: string) => r.problems.some((p) => p.kind === k)
    switch (f) {
      case 'all':
        return true
      case 'attention':
        return r.problems.some((p) => p.level === 'high' || p.level === 'medium')
      case 'unpushed':
        return has('unpushed')
      case 'ciRed':
        return has('ciFailing')
      case 'alerts':
        return has('alerts')
      case 'notCloned':
        return has('notCloned')
      case 'localOnly':
        return has('localOnly')
    }
  }

  const rows = $derived.by(() => {
    if (!board) return []
    const q = query.trim().toLowerCase()
    const list = board.rows.filter(
      (r) =>
        matches(r, filter) &&
        (!q ||
          r.name.toLowerCase().includes(q) ||
          (r.description ?? '').toLowerCase().includes(q) ||
          r.clones.some((c) => c.path.toLowerCase().includes(q))),
    )
    const by: Record<Sort, (a: Row, b: Row) => number> = {
      worst: () => 0,
      name: (a, b) => a.name.localeCompare(b.name),
      pushed: (a, b) => (b.pushedAt ?? '').localeCompare(a.pushedAt ?? ''),
      release: (a, b) => (b.release?.publishedAt ?? '').localeCompare(a.release?.publishedAt ?? ''),
    }
    return sort === 'worst' ? list : [...list].sort(by[sort])
  })

  function count(f: Filter): number {
    if (!board) return 0
    const c = board.counts
    return { all: c.all, attention: c.attention, unpushed: c.unpushed, ciRed: c.ciRed, alerts: c.alerts, notCloned: c.notCloned, localOnly: c.localOnly, hidden: c.hidden }[f]
  }

  export function focusSearch() {
    search?.focus()
  }

  export async function load() {
    try {
      board = await api.board()
      if (board.counts.attention === 0 && filter === 'attention') filter = 'all'
    } catch (e) {
      error = String(e)
    }
  }

  export async function refresh(fetch = false) {
    if (busy) return
    busy = fetch ? 'fetch' : 'refresh'
    error = ''
    try {
      board = await api.refresh(fetch)
    } catch (e) {
      error = String(e)
    } finally {
      busy = ''
    }
  }

  async function hide(row: Row, hidden: boolean) {
    await api.setHidden(row.key, hidden)
    await load()
    note = hidden ? `${row.name} is hidden. Settings lists hidden rows.` : `${row.name} is back on the board.`
  }

  async function act(row: Row) {
    if (!row.action) return
    try {
      await runAction(row.action)
    } catch (e) {
      error = String(e)
    }
  }

  function localLine(r: Row): string {
    if (!r.clones.length) return ''
    const c = r.clones[0]
    if (c.error) return 'unreadable'
    const parts = [c.branch ?? 'detached']
    if (c.upstream && (c.ahead || c.behind)) parts.push(`↑${c.ahead} ↓${c.behind}`)
    if (c.changed) parts.push(`${c.changed} changed`)
    return parts.join(' · ')
  }

  onMount(() => {
    load().then(() => {
      // Read GitHub at start when the last answer is older than ten minutes.
      if (!board?.checkedAt || Date.now() / 1000 - board.checkedAt > 600) refresh()
    })
    const t = setInterval(() => tick++, 30_000)
    return () => clearInterval(t)
  })
</script>


  <div class="bar">
    <div class="filters caps" role="tablist" aria-label="Filter">
      {#each filters as f (f.id)}
        <button role="tab" aria-selected={filter === f.id} class:on={filter === f.id} onclick={() => (filter = f.id)}>
          {f.label} <span class="n" class:hot={f.id !== 'all' && f.id !== 'notCloned' && count(f.id) > 0}>{count(f.id)}</span>
        </button>
      {/each}
      {#if board && board.counts.hidden}
        <button role="tab" aria-selected={filter === 'hidden'} class:on={filter === 'hidden'} onclick={() => (filter = 'hidden')}>
          Hidden <span class="n">{board.counts.hidden}</span>
        </button>
      {/if}
    </div>
    <div class="spacer"></div>
    <label class="caps sortlabel" for="repo-sort">Sort</label>
    <select id="repo-sort" bind:value={sort}>
      <option value="worst">Worst first</option>
      <option value="pushed">Recently pushed</option>
      <option value="release">Latest release</option>
      <option value="name">Name</option>
    </select>
    <input bind:this={search} type="search" placeholder="Search repositories  /" bind:value={query} aria-label="Search repositories" />
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
      <p class="empty">Looking at your clones…</p>
    {:else if !rows.length}
      <p class="empty">
        {query ? 'No repository matches.' : filter === 'attention' ? 'Nothing needs attention.' : 'Nothing here.'}
        {#if !board.checkedAt}<br />GitHub has not been read yet: press R or the refresh button.{/if}
      </p>
    {:else if view === 'table'}
      <table class="board">
        <thead class="caps">
          <tr>
            <th class="name">Repository</th>
            <th>Status</th>
            <th>Local</th>
            <th>Checks</th>
            <th>Release</th>
            <th>Alerts</th>
            <th>Pushed</th>
            <th class="act"><span class="sr">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          {#each rows as r (r.key)}
            {@const c = chip(r)}
            <tr class="row" class:open={open === r.key} onclick={() => (open = open === r.key ? null : r.key)}>
              <td class="name"><div class="namecell">
                <button class="expand" aria-expanded={open === r.key} aria-label="Details for {r.name}" onclick={(e) => (e.stopPropagation(), (open = open === r.key ? null : r.key))}>
                  {open === r.key ? '▾' : '▸'}
                </button>
                <div>
                  <div class="title">
                    {r.name}
                    {#if r.private}<span class="tag caps">private</span>{/if}
                    {#if r.foreign}<span class="tag caps">upstream</span>{/if}
                    {#if r.clones.length > 1}<span class="tag caps">{r.clones.length} clones</span>{/if}
                  </div>
                  <div class="desc">{r.description ?? (r.clones[0] ? folderName(r.clones[0].path) : '')}</div>
                </div>
              </div></td>
              <td>
                <span class="chip caps tone-{c.tone}">{c.label}</span>
                {#if r.problems.length > 1}<span class="more mono">+{r.problems.length - 1}</span>{/if}
              </td>
              <td class="mono small">{localLine(r) || '—'}</td>
              <td><span class="dot {r.ci}" aria-hidden="true"></span><span class="small">{{ passing: 'Passing', failing: 'Failing', running: 'Running', none: '—' }[r.ci]}</span></td>
              <td class="mono small">
                {#if r.release}
                  <span class:pre={r.release.prerelease} title={r.release.prerelease ? 'Pre-release' : 'Release'}>{r.release.tag}</span>
                  <div class="desc">{shortDate(r.release.publishedAt)}</div>
                {:else}—{/if}
              </td>
              <td class="mono small">
                {#if r.alerts && r.alerts.total}
                  <span class={r.alerts.critical + r.alerts.high ? 'bad' : 'warn'}>{r.alerts.total}</span>
                  {#if r.alerts.critical + r.alerts.high}<span class="desc"> {r.alerts.critical + r.alerts.high} serious</span>{/if}
                {:else}—{/if}
              </td>
              <td class="small muted" data-tick={tick}>{ago(r.pushedAt)}</td>
              <td class="act" onclick={(e) => e.stopPropagation()}><div class="actcell">
                {#if r.action}
                  <button class="btn caps primary" class:quiet={!r.problems.length || r.problems[0].level === 'info'} onclick={() => act(r)}>{r.action.label}</button>
                {/if}
                <RowMenu row={r} onhide={(h) => hide(r, h)} onerror={(m) => (error = m)} />
              </div></td>
            </tr>
            {#if open === r.key}
              <tr class="detailrow"><td colspan="8"><Detail row={r} roots={board.roots} /></td></tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {:else}
      <div class="grid">
        {#each rows as r (r.key)}
          {@const c = chip(r)}
          <article class="card">
            <div class="cardtop caps">
              <span class="chip tone-{c.tone}">{c.label}</span>
              {#if r.problems.length > 1}<span class="more">+{r.problems.length - 1}</span>{/if}
              <span class="spacer"></span>
              {#if r.private}<span class="tag">private</span>{/if}
            </div>
            <h3>{r.name}</h3>
            <p class="desc clamp">{r.description ?? (r.clones[0] ? folderName(r.clones[0].path) : 'No description')}</p>
            <p class="facts small">
              <span class="dot {r.ci}" aria-hidden="true"></span>{{ passing: 'Checks passing', failing: 'Checks failing', running: 'Checks running', none: 'No checks' }[r.ci]}
              · {r.release ? `${r.release.tag}${r.release.prerelease ? ' (pre)' : ''}` : 'no release'}
              {#if r.clones.length} · <span class="mono">{localLine(r)}</span>{/if}
            </p>
            {#if r.problems[0]}<p class="first small">{r.problems[0].text}</p>{/if}
            <div class="cardact">
              {#if r.action}
                <button class="btn caps primary" class:quiet={!r.problems.length || r.problems[0].level === 'info'} onclick={() => act(r)}>{r.action.label}</button>
              {/if}
              <RowMenu row={r} onhide={(h) => hide(r, h)} onerror={(m) => (error = m)} />
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </main>

  <footer class="foot caps" data-tick={tick}>
    <span>Checked {board?.checkedAt ? ago(board.checkedAt) : 'never'}{board?.login ? ` · ${board.login}` : ''}</span>
    {#if board?.rate}
      <span class:lowbudget={board.rate.remaining < 500}>GitHub budget {board.rate.remaining.toLocaleString()}/{board.rate.limit.toLocaleString()}</span>
    {/if}
    {#if board?.warnings.length}
      <button class="linkish caps" onclick={() => (showWarnings = !showWarnings)}>{plural(board.warnings.length, 'note', 'notes')}</button>
    {/if}
    <span class="spacer"></span>
    <span>{rows.length} shown</span>
    <span>{board?.platform ?? ''}</span>
  </footer>
  {#if showWarnings && board?.warnings.length}
    <ul class="warnings small">
      {#each board.warnings as w, i (i)}<li>{w}</li>{/each}
    </ul>
  {/if}
