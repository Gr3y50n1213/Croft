<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { PRIVACY_POLICY_URL, SOURCE_URL } from './links.ts';

  let version = $state('');

  onMount(() => {
    fetch('/version', { cache: 'no-store', credentials: 'omit' })
      .then((r) => (r.ok ? r.text() : ''))
      .then((t) => (version = t.trim().slice(0, 40)))
      .catch(() => {});
  });
</script>

<section class="pane" aria-labelledby="about-title">
  <h2 id="about-title">About</h2>
  <dl>
    <dt>Companion version</dt>
    <dd data-testid="version">{version || '…'}</dd>
    <dt>Licence</dt>
    <dd>
      This web app is free software under the Mozilla Public License 2.0. The Companion is under the GNU Affero
      General Public License 3.0.
    </dd>
    <dt>Privacy</dt>
    <dd>
      Your mail stays between this browser, your Companion and your mail server. Croft has no analytics and no
      tracking. <a href={PRIVACY_POLICY_URL} target="_blank" rel="noopener noreferrer">Privacy policy</a>
    </dd>
    <dt>Source code</dt>
    <dd><a href={SOURCE_URL} target="_blank" rel="noopener noreferrer">Source code</a></dd>
  </dl>
</section>

<style>
  .pane {
    max-width: 720px;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 22px;
  }

  dt {
    margin-top: 16px;
    font-weight: 600;
  }

  dd {
    margin: 4px 0 0;
  }

  a {
    color: var(--croft-color-primary);
  }
</style>
