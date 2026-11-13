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
    keyboard: 'M3 7h18v10H3zM7 11h.5M11 11h.5M15 11h.5M8 14h8',
    settings:
      'M12 9a3 3 0 1 1 0 6 3 3 0 0 1 0-6zM12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1',
    account: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 20c1-4 4.5-6 8-6s7 2 8 6',
    swipe: 'M4 12h16M15 7l5 5-5 5M9 7l-5 5 5 5',
    palette:
      'M12 3a9 9 0 1 0 0 18c1 0 1.5-.8 1.5-1.5 0-1-.8-1.4-.8-2.3 0-.9.7-1.7 1.7-1.7H17a4 4 0 0 0 4-4c0-4.7-4-8.5-9-8.5zM7.5 11h.5M10 7h.5M14.5 7h.5',
    lock: 'M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3',
    clock: 'M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 7v5l3 2',
    bell: 'M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15zM10 20.5a2 2 0 0 0 4 0',
    info: 'M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 11v6M12 7.5v.5'
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
