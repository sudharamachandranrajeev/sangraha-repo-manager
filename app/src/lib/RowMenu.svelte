<script lang="ts">
  import { api, type Row } from './api'
  import { folderName } from './format'

  let { row, onhide, onerror }: { row: Row; onhide: (hidden: boolean) => void; onerror: (msg: string) => void } =
    $props()

  let open = $state(false)
  let root: HTMLDivElement

  type Item = { label: string; run: () => Promise<unknown> | void }

  const items = $derived.by(() => {
    const list: Item[] = []
    const url = row.url
    if (url) {
      list.push({ label: 'Open on GitHub', run: () => api.openUrl(url) })
      if (!row.foreign) {
        list.push({ label: 'Checks', run: () => api.openUrl(`${url}/actions`) })
        list.push({ label: 'Security alerts', run: () => api.openUrl(`${url}/security/dependabot`) })
        list.push({ label: 'Releases', run: () => api.openUrl(`${url}/releases`) })
        list.push({ label: 'Pull requests', run: () => api.openUrl(`${url}/pulls`) })
      }
    }
    for (const c of row.clones) {
      const name = row.clones.length > 1 ? ` (${folderName(c.path)})` : ''
      list.push({ label: `Open folder${name}`, run: () => api.openFolder(c.path) })
      list.push({ label: `Open terminal${name}`, run: () => api.openTerminal(c.path) })
      list.push({ label: `Copy path${name}`, run: () => navigator.clipboard.writeText(c.path) })
    }
    list.push({ label: row.hidden ? 'Show on the board' : 'Hide from the board', run: () => onhide(!row.hidden) })
    return list
  })

  async function pick(item: Item) {
    open = false
    try {
      await item.run()
    } catch (e) {
      onerror(String(e))
    }
  }

  function outside(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false
  }

  function key(e: KeyboardEvent) {
    if (open && e.key === 'Escape') {
      open = false
      ;(root.querySelector('button') as HTMLButtonElement | null)?.focus()
    }
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={key} />

<div class="menu" bind:this={root}>
  <button
    class="btn ghost icon"
    aria-label="More actions for {row.name}"
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={(e) => {
      e.stopPropagation()
      open = !open
    }}>…</button
  >
  {#if open}
    <div class="pop" role="menu">
      {#each items as item (item.label)}
        <button role="menuitem" onclick={(e) => (e.stopPropagation(), pick(item))}>{item.label}</button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
  }
  .pop {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 20;
    min-width: 220px;
    max-height: 340px;
    overflow: auto;
    padding: 4px;
    border: 1px solid var(--line-strong);
    border-radius: 3px;
    background: var(--surface);
    box-shadow: 0 10px 30px rgb(0 0 0 / 18%);
    transform-origin: top right;
    animation: pop 140ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.97);
    }
  }
  .pop button {
    display: block;
    width: 100%;
    padding: 7px 10px;
    border: 0;
    border-radius: 2px;
    background: none;
    text-align: left;
  }
  .pop button:hover,
  .pop button:focus-visible {
    background: var(--surface-2);
    outline: none;
  }
</style>
