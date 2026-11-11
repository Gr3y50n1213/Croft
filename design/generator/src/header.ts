// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The MPL-2.0 "Exhibit A" notice, in the comment styles the generated files need.

const LINES = [
  'This Source Code Form is subject to the terms of the Mozilla Public',
  'License, v. 2.0. If a copy of the MPL was not distributed with this',
  'file, You can obtain one at https://mozilla.org/MPL/2.0/.',
];

export const MPL_HEADER_SLASH = LINES.map((l) => `// ${l}`).join('\n');
export const MPL_HEADER_CSS = `/*\n${LINES.map((l) => ` * ${l}`).join('\n')}\n */`;
