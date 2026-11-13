<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<svelte:head><title>Companion data deleted · Croft</title></svelte:head>

<main class="deleted" data-testid="deleted">
  <h1>Your Companion data is deleted</h1>
  <p>
    Your account on this Companion is gone: its copy of your mail, your mail accounts and their passwords, your passkeys, your
    signed-in devices and their notifications. Your mail on your mail servers is not touched, and neither is the mail on your
    phone.
  </p>
  <p>The Croft app on your phone signs out of this Companion the next time it reaches it.</p>
</main>

<style>
  .deleted {
    max-width: 560px;
    margin: 64px auto;
    padding: 0 16px;
  }
</style>
