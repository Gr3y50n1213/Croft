<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts" module>
  /** Simple 24 px line icons (drawn for Croft), by name. */
  const PATHS = {
    archive: 'M4 5h16v4H4zM5 9v10h14V9M10 13h4',
    delete: 'M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12M10 11v5M14 11v5',
    read: 'M4 9l8 5 8-5M4 9v10h16V9l-8-5z',
    unread: 'M4 6h16v12H4zM4 7l8 6 8-6',
    star: 'M12 4l2.4 5 5.6.6-4.2 3.8 1.2 5.6L12 16.2 7 19l1.2-5.6L4 9.6 9.6 9z',
    folder: 'M4 6h6l2 2h8v10H4z',
    spam: 'M12 3l8 4v5c0 4.5-3.4 8.2-8 9-4.6-.8-8-4.5-8-9V7zM12 8v5M12 16v.5',
    inbox: 'M4 5h16v14H4zM4 13h5l1 2h4l1-2h5',
    sent: 'M4 12l16-7-6 16-3-7z',
    drafts: 'M6 3h9l4 4v14H6zM14 3v5h5',
    outbox: 'M12 15V4M8 8l4-4 4 4M5 14v6h14v-6',
    trash: 'M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12',
    compose: 'M5 19l1-4L16 5l3 3L9 18zM14 7l3 3',
    back: 'M19 12H5M11 6l-6 6 6 6',
    close: 'M6 6l12 12M18 6L6 18',
    refresh: 'M19 8a8 8 0 1 0 1 6M19 4v4h-4',
    search: 'M10.5 4a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM15.5 15.5L20 20',
    keyboard: 'M3 7h18v10H3zM7 11h.5M11 11h.5M15 11h.5M8 14h8'
  } as const;
  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  let { name, filled = false, size = 20 }: { name: IconName; filled?: boolean; size?: number } = $props();
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  aria-hidden="true"
  fill={filled ? 'currentColor' : 'none'}
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"><path d={PATHS[name]} /></svg>

<style>
  .icon {
    flex: none;
    display: block;
  }
</style>
