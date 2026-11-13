// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The look chosen in W04 (stored in this browser), set before the first paint so a reload never
// flashes the default look. The same rules as src/lib/settings/appearance.ts (`attributesOf`),
// kept to plain old JavaScript: a file, as the page's CSP allows no inline script but SvelteKit's.
// The app takes over once it starts (and follows later changes); this only paints the first frame.
(function () {
  'use strict';
  var allowed = {
    mode: ['system', 'light', 'dark'],
    palette: ['moss', 'wax', 'slate', 'heather', 'ochre', 'peat', 'tide', 'rowan'],
    contrast: ['standard', 'medium', 'high'],
    density: ['comfortable', 'default', 'compact'],
    roundness: ['tight', 'standard', 'round'],
    list: ['container', 'flat'],
    motion: ['system', 'full', 'reduced', 'off']
  };
  var a = {};
  try {
    var parsed = JSON.parse(window.localStorage.getItem('croft.appearance') || '{}');
    if (parsed && typeof parsed === 'object') a = parsed;
  } catch (e) {
    a = {};
  }
  var pick = function (k, fallback) {
    return allowed[k].indexOf(a[k]) >= 0 ? a[k] : fallback;
  };
  var mode = pick('mode', 'system');
  var motion = pick('motion', 'system');
  var dark = mode === 'dark' || (mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  if (motion === 'system') motion = window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'reduced' : 'full';
  var root = document.documentElement;
  root.setAttribute('data-croft-theme', pick('palette', 'moss'));
  root.setAttribute('data-croft-mode', dark ? (a.pureBlack === true ? 'pure-black' : 'dark') : 'light');
  root.setAttribute('data-croft-contrast', pick('contrast', 'standard'));
  root.setAttribute('data-croft-density', pick('density', 'default'));
  root.setAttribute('data-croft-roundness', pick('roundness', 'standard'));
  root.setAttribute('data-croft-list', pick('list', 'container'));
  root.setAttribute('data-croft-motion', motion);
})();
