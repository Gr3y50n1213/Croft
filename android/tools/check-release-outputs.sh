#!/usr/bin/env bash
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
#
# Checks the release outputs: unsigned, not debuggable, no dependency-info block (F-Droid rejects it),
# the merged manifest matches tools/manifest-allowlist.json, and 16 KB page alignment (APK and, when bundletool
# is given, the AAB config). Run from android/ after `./gradlew :app:assembleRelease :app:bundleRelease`.
# Usage: tools/check-release-outputs.sh [path/to/bundletool.jar]
#
# With CROFT_VARIANT=preview it checks the preview APK instead (after `./gradlew :app:assemblePreview`): signed with the debug
# key (not unsigned, and not any other key), not debuggable, the same manifest allowlist, 16 KB alignment, no test-server code and
# no debug test hooks. There is no AAB for preview, so the unsigned, dependency-blob and bundle checks do not apply.
set -euo pipefail
cd "$(dirname "$0")/.."
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:?set ANDROID_HOME}}"
bt="$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)"
variant="${CROFT_VARIANT:-release}"
case "$variant" in
  release)
    apk=app/build/outputs/apk/release/app-release-unsigned.apk
    aab=app/build/outputs/bundle/release/app-release.aab
    [ -f "$apk" ] || { echo "FAIL missing $apk (is the release APK built unsigned?)"; exit 1; }
    [ -f "$aab" ] || { echo "FAIL missing $aab"; exit 1; }
    ;;
  preview)
    apk=app/build/outputs/apk/preview/app-preview.apk
    aab=""
    [ -f "$apk" ] || { echo "FAIL missing $apk"; exit 1; }
    ;;
  *) echo "FAIL unknown CROFT_VARIANT: $variant"; exit 1 ;;
esac
fail=0
bad() { echo "FAIL $*"; fail=1; }

if [ "$variant" = release ]; then
  # Unsigned: no signature files, no APK signing block (which is also where AGP's dependency blob would go).
  if unzip -Z1 "$apk" | grep -Eq '^META-INF/[^/]+\.(RSA|DSA|EC|SF)$'; then bad "APK carries a v1 signature"; else echo "ok   APK has no v1 signature"; fi
  if grep -aq 'APK Sig Block 42' "$apk"; then bad "APK has a signing block"; else echo "ok   APK has no signing block (so no dependency blob)"; fi
  # Dependency info as a file in the bundle metadata.
  if unzip -Z1 "$aab" | grep -q 'dependencies.pb'; then bad "AAB has dependencies.pb (dependenciesInfo.includeInBundle must be false)"; else echo "ok   AAB has no dependencies.pb"; fi
  if unzip -Z1 "$aab" | grep -Eq '^META-INF/[^/]+\.(RSA|DSA|EC|SF)$'; then bad "AAB is signed"; else echo "ok   AAB is unsigned"; fi
else
  # Preview: signed, and with the standard Android debug key (never an owner's release key). The debug certificate's subject is
  # "CN=Android Debug, O=Android, C=US".
  signer="$(ls "$bt"/apksigner 2>/dev/null || true)"
  [ -n "$signer" ] || bad "apksigner not found in $bt"
  if [ -n "$signer" ]; then
    certs="$("$signer" verify --print-certs "$apk" 2>&1)" || { bad "preview APK signature does not verify: $certs"; certs=""; }
    if grep -q 'CN=Android Debug' <<<"$certs"; then echo "ok   preview APK is signed with the Android debug key"; else bad "preview APK is not signed with the Android debug key"; fi
  fi
fi

# Not debuggable, from the compiled manifest of the APK itself.
if "$bt/aapt2" dump xmltree --file AndroidManifest.xml "$apk" | grep -q 'debuggable'; then bad "APK manifest sets android:debuggable"; else echo "ok   APK is not debuggable"; fi

# Permissions and exported components, on the merged manifest AGP packaged into the APK.
vcap="$(tr '[:lower:]' '[:upper:]' <<<"${variant:0:1}")${variant:1}"
python3 tools/check-release-manifest.py "app/build/intermediates/packaged_manifests/$variant/process${vcap}ManifestForPackage/AndroidManifest.xml" || fail=1

# 16 KB alignment: the libraries, then the whole APK.
rm -rf "app/build/$variant-libs" && unzip -q "$apk" 'lib/*' -d "app/build/$variant-libs"
tools/check-16kb-alignment.sh "app/build/$variant-libs" || fail=1
# T16: the Rust library of the release must not carry the test-only IMAP server (`test-server` feature of croft-ffi, which is for
# the host library of the JVM tests). Its UniFFI object exports symbols named after `TestImapServer`, and its crate is
# `croft-test-support`; neither may appear in any library of the APK, as an exported symbol or as text anywhere in the file.
nm_tool="$(ls "$sdk"/ndk/*/toolchains/llvm/prebuilt/*/bin/llvm-nm 2>/dev/null | sort -V | tail -1 || true)"
[ -n "$nm_tool" ] || nm_tool="$(command -v llvm-nm || command -v nm || true)"
libs=$(find "app/build/$variant-libs/lib" -name '*.so' | sort)
[ -n "$libs" ] || bad "no libraries found in the release APK"
for lib in $libs; do
  syms=""; [ -z "$nm_tool" ] || syms="$("$nm_tool" -D "$lib" 2>/dev/null || true)"
  if grep -qi 'testimapserver' <<<"$syms"; then
    bad "$lib exports test-server symbols (TestImapServer): the release library was built with the test-server feature"
  elif grep -aqiE 'testimapserver|croft[-_]test[-_]support|test-support/src' "$lib"; then
    bad "$lib contains the test server (TestImapServer / croft-test-support): the release library was built with the test-server feature"
  else
    echo "ok   $lib has no test-server symbols (checked with ${nm_tool:-grep only})"
  fi
done

# T17c: the test hooks (`TestHooks` in src/debug, a stub in src/release) must not be in the release build. The debug class has a field
# named debugOnlySecretsFactory that exists nowhere else; R8's mapping lists the original names of what the release build kept.
mapping="app/build/outputs/mapping/$variant/mapping.txt"
if [ -f "$mapping" ]; then
  if grep -q 'debugOnlySecretsFactory' "$mapping"; then bad "the release build contains the debug test hooks (debugOnlySecretsFactory)"; else echo "ok   no debug test hooks in the release build"; fi
else
  # The release build is minified (isMinifyEnabled in app/build.gradle.kts), so R8 always writes a mapping; a missing one means this
  # check could not look, which must not pass quietly.
  bad "no R8 mapping at $mapping: cannot check that the debug test hooks are absent from the release build"
fi

"$bt/zipalign" -c -P 16 4 "$apk" && echo "ok   zipalign -c -P 16 on the APK" || bad "zipalign -P 16"

if [ "$variant" != release ]; then
  echo "skip AAB 16 KB config (no AAB for $variant)"
elif [ $# -ge 1 ]; then
  if java -jar "$1" dump config --bundle="$aab" | grep -q 'PAGE_ALIGNMENT_16K'; then echo "ok   AAB config has PAGE_ALIGNMENT_16K"; else bad "AAB config lacks PAGE_ALIGNMENT_16K"; fi
else
  echo "skip AAB 16 KB config (no bundletool jar given)"
fi
exit $fail
