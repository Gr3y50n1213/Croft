# T16b: Release build

Crate / folder: `android/` and `.github/workflows/`

Depends on: T07 (the Gradle project), T16 (so the release build has real code to shrink). Blocks: any store or F-Droid submission.

## Read first
`docs/store/READINESS.md` (Google Play checklist: target API, 16 KB pages, permissions, signing), `docs/store/AUDIT-2026-11-11.md` (gaps 1 to 4), `android/app/build.gradle.kts`, `android/tools/check-16kb-alignment.sh`, `.github/workflows/ci.yml`, `docs/OPEN-ITEMS.md`.

## Goal
`./gradlew :app:bundleRelease :app:assembleRelease` produces an unsigned, shrunk, reproducible-friendly release build in CI, and CI fails if the release manifest gains a permission or exported component nobody approved.

## Spec
- A `release` build type: R8 minify on, resource shrinking on, `isDebuggable = false`, no debug flags or test-only code paths. Keep rules in `proguard-rules.pro` for UniFFI/JNA and anything reflection-based, each with a one-line reason.
- `dependenciesInfo { includeInApk = false; includeInBundle = false }` (F-Droid rejects AGP's encrypted dependency blob; it also breaks reproducible builds).
- Signing keys are the owner's and never in the repository. The release build type has no signing config; CI builds unsigned. A local signing config may read keys from environment variables or a file outside the repo, and must be absent when they are not set.
- CI job: build `:app:bundleRelease` and `:app:assembleRelease` (unsigned), upload nothing public.
- Manifest check: a test or script that reads the release merged manifest and fails on any `uses-permission` or any exported component (activity, service, receiver, provider) not on an allowlist kept in the repository. The allowlist starts with INTERNET and the launcher activity only (AndroidX's internal signature permission and `ProfileInstallReceiver` guarded by DUMP are listed explicitly), and grows with T17 to T21 as each adds a permission from the READINESS table.
- 16 KB alignment: run `android/tools/check-16kb-alignment.sh` and `zipalign -c -P 16` on the release APK, and check the AAB with `bundletool dump config` for `PAGE_ALIGNMENT_16K`.
- Owner item (add to `docs/OPEN-ITEMS.md`): the application ID `com.example.croft` must be replaced by the owner's real ID before the first Play upload, because it cannot change afterwards and Play rejects `com.example` IDs. It is defined in `android/gradle.properties`, but Kotlin `package` lines and `CroftTokens.kt` also carry it: rename in one commit. Do not change it in this task.

## Done when
- `:app:bundleRelease` and `:app:assembleRelease` pass in CI and the outputs are unsigned.
- The manifest check passes on the current release manifest, and a test adds a fake permission and a fake exported receiver to a copy of the manifest and sees it fail for each.
- The 16 KB script and `zipalign -c -P 16` pass on the release APK; the bundle config shows 16 KB alignment.
- A test or script confirms the release APK has no dependency-info signing block (`dependenciesInfo` off) and `android:debuggable` is not set.
- On a device (not checkable here): the shrunk release build starts, signs in and syncs against a test server (R8 can break reflection and JNA); run by hand before any upload.

## Status (done)
- Built: release build type, `proguard-rules.pro`, `checkReleaseKeepRules` and `checkReleaseMapping`, `tools/check-release-manifest.py` with `tools/manifest-allowlist.json` and its tests, `tools/check-release-outputs.sh`, CI job `android-release`. See `android/README.md`, "Release build".
- The allowlist starts with no permission of the app's own (the app declares none today, so INTERNET is not listed: T17 adds it). It lists AndroidX's signature-level `DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION` and `ProfileInstallReceiver` (guarded by DUMP).
- Not done here: the `docs/OPEN-ITEMS.md` application-ID item (outside this task's edit scope; the README documents it) and the on-device run of the shrunk build.
