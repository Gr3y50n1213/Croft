#!/usr/bin/env bash
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
#
# Checks the release outputs: unsigned, not debuggable, no dependency-info block (F-Droid rejects it),
# the merged manifest matches tools/manifest-allowlist.json, and 16 KB page alignment (APK and, when bundletool
# is given, the AAB config). Run from android/ after `./gradlew :app:assembleRelease :app:bundleRelease`.
# Usage: tools/check-release-outputs.sh [path/to/bundletool.jar]
set -euo pipefail
cd "$(dirname "$0")/.."
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:?set ANDROID_HOME}}"
bt="$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)"
apk=app/build/outputs/apk/release/app-release-unsigned.apk
aab=app/build/outputs/bundle/release/app-release.aab
[ -f "$apk" ] || { echo "FAIL missing $apk (is the release APK built unsigned?)"; exit 1; }
[ -f "$aab" ] || { echo "FAIL missing $aab"; exit 1; }
fail=0
bad() { echo "FAIL $*"; fail=1; }

# Unsigned: no signature files, no APK signing block (which is also where AGP's dependency blob would go).
if unzip -Z1 "$apk" | grep -Eq '^META-INF/[^/]+\.(RSA|DSA|EC|SF)$'; then bad "APK carries a v1 signature"; else echo "ok   APK has no v1 signature"; fi
if grep -aq 'APK Sig Block 42' "$apk"; then bad "APK has a signing block"; else echo "ok   APK has no signing block (so no dependency blob)"; fi
# Dependency info as a file in the bundle metadata.
if unzip -Z1 "$aab" | grep -q 'dependencies.pb'; then bad "AAB has dependencies.pb (dependenciesInfo.includeInBundle must be false)"; else echo "ok   AAB has no dependencies.pb"; fi
if unzip -Z1 "$aab" | grep -Eq '^META-INF/[^/]+\.(RSA|DSA|EC|SF)$'; then bad "AAB is signed"; else echo "ok   AAB is unsigned"; fi

# Not debuggable, from the compiled manifest of the APK itself.
if "$bt/aapt2" dump xmltree --file AndroidManifest.xml "$apk" | grep -q 'debuggable'; then bad "APK manifest sets android:debuggable"; else echo "ok   APK is not debuggable"; fi

# Permissions and exported components, on the merged manifest AGP packaged into the APK.
python3 tools/check-release-manifest.py app/build/intermediates/packaged_manifests/release/processReleaseManifestForPackage/AndroidManifest.xml || fail=1

# 16 KB alignment: the libraries, then the whole APK.
rm -rf app/build/release-libs && unzip -q "$apk" 'lib/*' -d app/build/release-libs
tools/check-16kb-alignment.sh app/build/release-libs || fail=1
# T16: the Rust library of the release must not carry the test-only IMAP server (`test-server` feature of croft-ffi, which is for
# the host library of the JVM tests). Its UniFFI object exports symbols named after `TestImapServer`, and its crate is
# `croft-test-support`; neither may appear in any library of the APK, as an exported symbol or as text anywhere in the file.
nm_tool="$(ls "$sdk"/ndk/*/toolchains/llvm/prebuilt/*/bin/llvm-nm 2>/dev/null | sort -V | tail -1 || true)"
[ -n "$nm_tool" ] || nm_tool="$(command -v llvm-nm || command -v nm || true)"
libs=$(find app/build/release-libs/lib -name '*.so' | sort)
[ -n "$libs" ] || bad "no libraries found in the release APK"
for lib in $libs; do
  syms=""; [ -z "$nm_tool" ] || syms="$("$nm_tool" -D "$lib" 2>/dev/null || true)"
  if grep -qi 'testimapserver' <<<"$syms"; then
    bad "$lib exports test-server symbols (TestImapServer): the release library was built with the test-server feature"
  elif grep -aqiE 'testimapserver|croft[-_]test[-_]support' "$lib"; then
    bad "$lib contains the test server (TestImapServer / croft-test-support): the release library was built with the test-server feature"
  else
    echo "ok   $lib has no test-server symbols (checked with ${nm_tool:-grep only})"
  fi
done

"$bt/zipalign" -c -P 16 4 "$apk" && echo "ok   zipalign -c -P 16 on the APK" || bad "zipalign -P 16"

if [ $# -ge 1 ]; then
  if java -jar "$1" dump config --bundle="$aab" | grep -q 'PAGE_ALIGNMENT_16K'; then echo "ok   AAB config has PAGE_ALIGNMENT_16K"; else bad "AAB config lacks PAGE_ALIGNMENT_16K"; fi
else
  echo "skip AAB 16 KB config (no bundletool jar given)"
fi
exit $fail
