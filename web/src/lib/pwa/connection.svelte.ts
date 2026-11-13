// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Whether the app is offline (T32b): the browser says it has no network, or the Companion could not
 * be reached on the last request (a network that is up but goes nowhere).
 */
class Connection {
  browserOnline = $state(true);
  companionReached = $state(true);
  #started = false;

  get offline(): boolean {
    return !this.browserOnline || !this.companionReached;
  }

  start(): void {
    if (this.#started || typeof window === 'undefined') return;
    this.#started = true;
    this.browserOnline = navigator.onLine;
    window.addEventListener('online', () => (this.browserOnline = true));
    window.addEventListener('offline', () => (this.browserOnline = false));
  }
}

export const connection = new Connection();
