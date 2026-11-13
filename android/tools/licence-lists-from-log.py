#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Writes the licence lists CI made (T34d) from its log: the lines `LICENCE-LIST <path> <gzip, base64>` that the android-release job's
step "Make the licence lists again and compare (S60)" prints when the checked-in lists differ.

Usage (from android/): gh run view <run id> --log-failed | python3 tools/licence-lists-from-log.py
Only paths under app/src/*/assets/licenses/ are written.
"""
import base64
import gzip
import io
import json
import re
import sys
from pathlib import Path

MAX_BYTES = 4 * 1024 * 1024
LINE = re.compile(r"LICENCE-LIST (app/src/[a-z]+/assets/licenses/[a-z-]+\.json) ([A-Za-z0-9+/=]+)\s*$")


def main() -> int:
    written = 0
    for line in sys.stdin:
        m = LINE.search(line)
        if not m:
            continue
        path, data = m.groups()
        with gzip.GzipFile(fileobj=io.BytesIO(base64.b64decode(data))) as f:
            raw = f.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            raise SystemExit(f"{path}: larger than {MAX_BYTES} bytes")
        text = raw.decode("utf-8")
        json.loads(text)  # must be a list the app can read
        Path(path).write_text(text, encoding="utf-8")
        print(f"wrote {path}")
        written += 1
    return 0 if written else 1


if __name__ == "__main__":
    sys.exit(main())
