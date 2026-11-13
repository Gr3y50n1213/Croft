<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  // The generated tokens, read in place (design/generated is never copied).
  import '../../../design/generated/css/croft-tokens.css';
  import '../app.css';
  import { onMount } from 'svelte';
  import { settings } from '#lib/settings/store.svelte.ts';

  let { children } = $props();

  // W04's look, applied by static/theme.js before the first paint, is followed from here on.
  onMount(() => settings.start());
</script>

{@render children()}
