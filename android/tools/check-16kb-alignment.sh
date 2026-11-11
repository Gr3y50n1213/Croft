#!/usr/bin/env bash
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
#
# Fails unless every LOAD segment of every native library under the given directory has an
# alignment of at least 0x4000 (16 KB pages; Google Play requirement, docs/store/READINESS.md).
# Usage: tools/check-16kb-alignment.sh app/build/generated/rust/jniLibs
set -euo pipefail

dir="${1:?usage: check-16kb-alignment.sh <directory with .so files>}"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
readelf="$(ls "$sdk"/ndk/*/toolchains/llvm/prebuilt/*/bin/llvm-readelf 2>/dev/null | sort | tail -1 || true)"
readelf="${readelf:-$(command -v llvm-readelf || true)}"
[ -n "$readelf" ] || { echo "llvm-readelf not found (set ANDROID_HOME)" >&2; exit 2; }

status=0
count=0
while IFS= read -r so; do
  count=$((count + 1))
  # The last column of a LOAD line in `readelf -lW` is the alignment, for example 0x4000.
  aligns="$("$readelf" -lW "$so" | awk '$1 == "LOAD" { print $NF }' | sort -u)"
  [ -n "$aligns" ] || { echo "FAIL $so: no LOAD segments"; status=1; continue; }
  ok=1
  for a in $aligns; do
    [ $((a)) -ge 16384 ] || ok=0
  done
  if [ "$ok" = 1 ]; then
    echo "ok   $so: LOAD align $(echo $aligns)"
  else
    echo "FAIL $so: LOAD segment alignment below 0x4000 ($(echo $aligns))"
    status=1
  fi
done < <(find "$dir" -name '*.so' | sort)
[ "$count" -gt 0 ] || { echo "no .so files under $dir" >&2; exit 2; }
exit "$status"
