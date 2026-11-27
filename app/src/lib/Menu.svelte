<script lang="ts" module>
  export type MenuItem = { label: string; run: () => Promise<unknown> | void }
</script>

<script lang="ts">
  /** The … button and its popover menu. */
  let { label, items, onerror }: { label: string; items: MenuItem[]; onerror: (msg: string) => void } = $props()

  let open = $state(false)
  let root: HTMLDivElement

  async function pick(item: MenuItem) {
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
    aria-label={label}
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
