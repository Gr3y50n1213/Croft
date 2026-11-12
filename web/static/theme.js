// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Light or dark follows the system (W04's Appearance comes in T32), set before the first paint.
(function () {
  'use strict';
  var q = window.matchMedia('(prefers-color-scheme: dark)');
  var set = function () {
    document.documentElement.setAttribute('data-croft-mode', q.matches ? 'dark' : 'light');
  };
  set();
  q.addEventListener('change', set);
})();
