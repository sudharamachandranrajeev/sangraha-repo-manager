<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type TidyBoard, type TidyRepo } from './api'
  import { ago, plural, size } from './format'

  let { active, busy = $bindable('') }: { active: boolean; busy?: '' | 'refresh' } = $props()

  let board = $state<TidyBoard | null>(null)
  let query = $state('')
  let error = $state('')
  let note = $state('')
  let search: HTMLInputElement
  let tick = $state(0)
  let showWarnings = $state(false)

  // The repository whose merged branches are about to be deleted, and which of them.
  let deleting = $state<TidyRepo | null>(null)
  let picked = $state<string[]>([])
  let working = $state('')

  const rows = $derived.by(() => {
    if (!board) return []
    const q = query.trim().toLowerCase()
    return board.repos.filter((r) => !q || r.repo.toLowerCase().includes(q))
  })

  function short(full: string): string {
    return full.split('/')[1] ?? full
  }

  const missingText: Record<string, string> = { README: 'No README', LICENSE: 'No licence', CI: 'No CI' }

  export function focusSearch() {
    search?.focus()
  }

  export async function load() {
    try {
      board = await api.tidyBoard()
    } catch (e) {
      error = String(e)
    }
  }

  export async function refresh() {
    if (busy) return
    busy = 'refresh'
    error = ''
    try {
      board = await api.tidyRefresh()
    } catch (e) {
      error = String(e)
    } finally {
      busy = ''
    }
  }

  function ask(r: TidyRepo) {
    deleting = r
    picked = r.merged.map((b) => b.name)
  }

  async function remove(r: TidyRepo, names: string[]) {
    deleting = null
    working = r.repo
    try {
      const out = await api.deleteMerged(r.repo, names)
      note = `${short(r.repo)}: ${out.join('; ')}`
      await load()
    } catch (e) {
      error = String(e)
    } finally {
      working = ''
    }
  }

  // GitHub is read when the tab is first opened and nothing was ever read; after that only on
  // Refresh (or by the daily digest once a week).
  let checked = false
  $effect(() => {
    if (!active || checked || !board) return
    checked = true
    if (!board.checkedAt) refresh()
  })

  onMount(() => {
    load()
    const t = setInterval(() => tick++, 30_000)
    return () => clearInterval(t)
  })
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (deleting = null)} />

<div class="bar">
  {#if board}
    {@const c = board.counts}
    <p class="tally caps small" aria-label="Housekeeping summary">
      <span class:zero={!c.merged} class="t-warn">{plural(c.merged, 'merged branch', 'merged branches')}</span>
      <span class:zero={!c.stale}>{plural(c.stale, 'stale branch', 'stale branches')}</span>
      <span class:zero={!c.noReadme}>{c.noReadme} without README</span>
      <span class:zero={!c.noLicense}>{c.noLicense} public without licence</span>
      <span class:zero={!c.noCi}>{c.noCi} without CI</span>
      <span class:zero={!c.oldRuns}>{plural(c.oldRuns, 'old run', 'old runs')}</span>
      <span class:zero={!c.largeFiles} class="t-bad">{plural(c.largeFiles, 'large file', 'large files')}</span>
    </p>
  {/if}
  <div class="spacer"></div>
  <input bind:this={search} type="search" placeholder="Search repos  /" bind:value={query} aria-label="Search repositories" />
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
  <p class="hint small">
    <em>Merged</em> branches went in through a pull request and have nothing newer, so deleting them loses nothing (GitHub can restore a deleted branch from the pull
    request). <em>Stale</em> branches have had no commit for 90 days and no pull request. Large files (20 MB or more) are read from your clones.
  </p>
  {#if !board}
    <p class="empty">Reading…</p>
  {:else if !rows.length}
    <p class="empty">{query ? 'No repository matches.' : board.checkedAt ? 'Nothing to tidy.' : 'Press Refresh to read GitHub.'}</p>
  {:else}
    <table class="board">
      <thead class="caps">
        <tr>
          <th class="name">Repository</th>
          <th>Branches</th>
          <th>Missing</th>
          <th>Old runs</th>
          <th>Large files</th>
          <th class="act"><span class="sr">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        {#each rows as r (r.repo)}
          <tr>
            <td class="name">
              <div class="title">{short(r.repo)}</div>
              {#if r.branches}<div class="desc">{plural(r.branches, 'branch', 'branches')} on GitHub</div>{/if}
            </td>
            <td class="small">
              {#if r.merged.length}
                <div title={r.merged.map((b) => `${b.name} (#${b.pullRequest})`).join('\n')}>
                  <span class="t-warn">{r.merged.length} merged</span>
                  <span class="muted mono">{r.merged.slice(0, 2).map((b) => b.name).join(', ')}{r.merged.length > 2 ? ' …' : ''}</span>
                </div>
              {/if}
              {#if r.stale.length}
                <div title={r.stale.map((b) => `${b.name}: last commit ${b.lastCommit?.slice(0, 10)}`).join('\n')}>
                  {r.stale.length} stale <span class="muted mono">{r.stale.slice(0, 2).map((b) => b.name).join(', ')}{r.stale.length > 2 ? ' …' : ''}</span>
                </div>
              {/if}
              {#if !r.merged.length && !r.stale.length}<span class="muted">—</span>{/if}
            </td>
            <td>
              <div class="chips">
                {#each r.missing as m (m)}<span class="chip caps tone-muted">{missingText[m] ?? m}</span>{/each}
              </div>
            </td>
            <td class="small">
              {#if r.oldRuns}
                <button class="linkish" title="Workflow runs older than 90 days" onclick={() => api.openUrl(`https://github.com/${r.repo}/actions`)}>{r.oldRuns}</button>
              {:else}<span class="muted">—</span>{/if}
            </td>
            <td class="small">
              {#each r.largeFiles.slice(0, 3) as f (f.path)}<div><span class="mono">{f.path}</span> <span class="muted">{size(f.size)}</span></div>{/each}
              {#if r.largeFiles.length > 3}<div class="muted">and {r.largeFiles.length - 3} more</div>{/if}
              {#if !r.largeFiles.length}<span class="muted">—</span>{/if}
            </td>
            <td class="act">
              <div class="actcell">
                {#if r.merged.length}
                  <button class="btn ghost caps" disabled={!!working || !!busy} onclick={() => ask(r)}>{working === r.repo ? 'Deleting…' : 'Delete merged'}</button>
                {/if}
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</main>

<footer class="foot caps" data-tick={tick}>
  <span>GitHub read {board?.checkedAt ? ago(board.checkedAt) : 'never'}</span>
  {#if board?.warnings.length}
    <button class="linkish caps" onclick={() => (showWarnings = !showWarnings)}>{plural(board.warnings.length, 'note', 'notes')}</button>
  {/if}
</footer>
{#if showWarnings && board?.warnings.length}
  <ul class="warnings small">
    {#each board.warnings as w, i (i)}<li>{w}</li>{/each}
  </ul>
{/if}

{#if deleting}
  {@const r = deleting}
  <div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (deleting = null)}>
    <div class="dialog wide" role="alertdialog" aria-modal="true" aria-labelledby="dm-title" aria-describedby="dm-text">
      <h2 id="dm-title" class="caps">Delete merged branches in {short(r.repo)}</h2>
      <p id="dm-text" class="small">
        These branches are deleted on GitHub. Each is read again first and kept if anything was pushed to it since. Your clones keep their own copies.
      </p>
      <ul class="plain small pick">
        {#each r.merged as b (b.name)}
          <li>
            <label class="check"
              ><input type="checkbox" value={b.name} bind:group={picked} /> <span class="mono">{b.name}</span>
              <span class="muted">pull request #{b.pullRequest}{b.lastCommit ? `, ${b.lastCommit.slice(0, 10)}` : ''}</span></label
            >
          </li>
        {/each}
      </ul>
      <div class="dlgact">
        <button class="btn ghost" onclick={() => (deleting = null)}>Cancel</button>
        <button class="btn caps primary" disabled={!picked.length} onclick={() => remove(r, picked)}>Delete {plural(picked.length, 'branch', 'branches')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    display: grid;
    gap: 16px;
    align-content: start;
  }
  h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }
  .muted {
    color: var(--muted);
  }
  .hint {
    margin: 0;
    color: var(--muted);
    max-width: 90ch;
  }
  .tally {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    margin: 0;
  }
  .t-warn {
    color: var(--warn);
  }
  .t-bad {
    color: var(--bad);
  }
  .zero {
    color: var(--faint);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .wide {
    width: min(640px, calc(100vw - 48px));
    max-height: calc(100vh - 48px);
    overflow: auto;
  }
  .plain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
  }
  .pick {
    margin: 8px 0;
    max-height: 320px;
    overflow: auto;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: baseline;
  }
</style>
