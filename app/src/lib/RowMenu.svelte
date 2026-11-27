<script lang="ts">
  import { api, type Row } from './api'
  import { folderName } from './format'
  import Menu, { type MenuItem } from './Menu.svelte'

  let { row, onhide, onerror }: { row: Row; onhide: (hidden: boolean) => void; onerror: (msg: string) => void } =
    $props()

  const items = $derived.by(() => {
    const list: MenuItem[] = []
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

</script>

<Menu label="More actions for {row.name}" {items} {onerror} />
