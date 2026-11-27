<script lang="ts">
  import Apps from './lib/Apps.svelte'
  import Bulk from './lib/Bulk.svelte'
  import type { Row } from './lib/api'
  import Repos from './lib/Repos.svelte'
  import Security from './lib/Security.svelte'
  import Settings from './lib/Settings.svelte'

  type Tab = 'repos' | 'apps' | 'security'
  type View = 'table' | 'grid'

  const saved = localStorage.getItem('tab')
  let tab = $state<Tab>(saved === 'apps' || saved === 'security' ? saved : 'repos')
  // Repos read best as a table, apps as cards; each tab keeps its own choice.
  let views = $state<Record<'repos' | 'apps', View>>({
    repos: localStorage.getItem('view') === 'grid' ? 'grid' : 'table',
    apps: localStorage.getItem('appsView') === 'table' ? 'table' : 'grid',
  })
  let repoBusy = $state<'' | 'refresh' | 'fetch'>('')
  let appBusy = $state<'' | 'refresh'>('')
  let secBusy = $state<'' | 'refresh'>('')
  let showSettings = $state(false)
  let bulkRows = $state<Row[] | null>(null)
  let repos = $state<Repos>()
  let apps = $state<Apps>()
  let security = $state<Security>()

  const busy = $derived(tab === 'repos' ? repoBusy : tab === 'apps' ? appBusy : secBusy)
  const pane = $derived(tab === 'repos' ? repos : tab === 'apps' ? apps : security)

  function refresh() {
    pane?.refresh()
  }

  function onKey(e: KeyboardEvent) {
    const typing = (e.target as HTMLElement).closest('input, textarea, select')
    if (typing || showSettings || bulkRows || e.ctrlKey || e.metaKey || e.altKey) return
    if (e.key === '/') {
      e.preventDefault()
      pane?.focusSearch()
    } else if (e.key === 'r') {
      refresh()
    } else if (e.key === '1' || e.key === '2' || e.key === '3') {
      tab = (['repos', 'apps', 'security'] as const)[Number(e.key) - 1]
    }
  }

  $effect(() => {
    localStorage.setItem('tab', tab)
    localStorage.setItem('view', views.repos)
    localStorage.setItem('appsView', views.apps)
  })
</script>

<svelte:window onkeydown={onKey} />

<div class="app">
  <header class="top">
    <div class="brand">
      <img src="/favicon.svg" alt="" width="22" height="22" />
      <span class="caps word">Sangraha</span>
    </div>
    <nav class="tabs caps" aria-label="Section">
      <button class="tab" class:on={tab === 'repos'} aria-current={tab === 'repos' ? 'page' : undefined} onclick={() => (tab = 'repos')}>Repos</button>
      <button class="tab" class:on={tab === 'apps'} aria-current={tab === 'apps' ? 'page' : undefined} onclick={() => (tab = 'apps')}>Apps</button>
      <button class="tab" class:on={tab === 'security'} aria-current={tab === 'security' ? 'page' : undefined} onclick={() => (tab = 'security')}>Security</button>
    </nav>
    <div class="spacer"></div>
    {#if tab !== 'security'}
      {@const t = tab}
      <div class="seg caps" role="group" aria-label="View">
        <button class:on={views[t] === 'table'} aria-pressed={views[t] === 'table'} onclick={() => (views[t] = 'table')}>Table</button>
        <button class:on={views[t] === 'grid'} aria-pressed={views[t] === 'grid'} onclick={() => (views[t] = 'grid')}>Grid</button>
      </div>
    {/if}
    {#if tab === 'repos'}
      <button class="btn ghost caps" onclick={() => (bulkRows = repos?.allRows() ?? [])} title="One change to many repositories, with a preview first">Bulk changes</button>
      <button class="btn ghost caps" disabled={!!busy} onclick={() => repos?.refresh(true)} title="git fetch in every clone, then read GitHub">
        {busy === 'fetch' ? 'Fetching…' : 'Fetch all'}
      </button>
    {/if}
    <button class="btn ghost icon" class:spin={busy === 'refresh'} disabled={!!busy} aria-label="Read GitHub again (R)" title="Read GitHub again (R)" onclick={refresh}
      ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
        ><path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v3h-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg
      ></button>
    <button class="btn ghost icon" aria-label="Settings" onclick={() => (showSettings = true)}
      ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
        ><path d="M2.5 4.5h11M2.5 11.5h11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /><circle cx="6" cy="4.5" r="1.8" fill="var(--surface)" stroke="currentColor" stroke-width="1.5" /><circle cx="10.5" cy="11.5" r="1.8" fill="var(--surface)" stroke="currentColor" stroke-width="1.5" /></svg
      ></button>
  </header>

  <!-- Both tabs stay mounted, so switching keeps filters, search and running installs. -->
  <div class="pane" hidden={tab !== 'repos'}>
    <Repos bind:this={repos} view={views.repos} bind:busy={repoBusy} />
  </div>
  <div class="pane" hidden={tab !== 'apps'}>
    <Apps bind:this={apps} view={views.apps} active={tab === 'apps'} bind:busy={appBusy} />
  </div>
  <div class="pane" hidden={tab !== 'security'}>
    <Security bind:this={security} active={tab === 'security'} bind:busy={secBusy} />
  </div>
</div>

{#if bulkRows}
  <Bulk rows={bulkRows} onclose={() => ((bulkRows = null), repos?.load())} />
{/if}

{#if showSettings}
  <Settings
    onclose={() => (showSettings = false)}
    onchange={() => {
      repos?.load()
      apps?.load()
      security?.load()
    }}
  />
{/if}

<style>
  .app {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .pane[hidden] {
    display: none;
  }
  .tab {
    border: 0;
  }
</style>
