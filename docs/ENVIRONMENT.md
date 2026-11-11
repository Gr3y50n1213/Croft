# Build environment

What the cloud container could and could not do. Checked 11 November 2026, first unattended session.

## Tools
| Tool | Result |
|---|---|
| Rust | 1.97.0 stable, with cargo, rustfmt, clippy |
| Python | 3.13.18 |
| Node.js / npm | 22.23.4 / 10.9.9 (Node 24 also present under `/opt/node24`) |
| Java | Present (used by Gradle) |
| Gradle | Present (`/opt/gradle-8.14.3`) |
| Docker | Installed. The daemon was not running at first; `dockerd` started fine by hand and pulled `greenmail/standalone:latest` |
| Android SDK | Not installed at first. Installed the command-line tools into `/opt/android-sdk` from dl.google.com, then platform-tools, `platforms;android-36`, `build-tools;36.0.0`, `ndk;29.0.14206865` |
| Android emulator | **Not possible**: no `/dev/kvm`, so no hardware acceleration. No device either. Anything that needs a real phone is "built, not checked on a device" |
| Xcode / iOS | Not possible (Linux) |

## Network
| Address | Result |
|---|---|
| index.crates.io and crate downloads | Works (`cargo fetch` downloaded a crate) |
| repo.maven.apache.org (Maven Central) | Single files work, but a full Gradle build gets **429 Too Many Requests** (rate limit on this container's address; the reply asked to retry after about 14 hours). See the note below |
| registry.npmjs.org | Works |
| services.gradle.org | Works |
| dl.google.com, maven.google.com (Android SDK, Google Maven) | Blocked at first; the owner allowed them on the environment during the session, then they worked |
| www.mozilla.org, www.gnu.org (licence texts) | Blocked at first; allowed during the session, then both licences downloaded |
| docs.rs | Blocked at first; allowed during the session |
| Docker Hub | Works (image pull) |

## Machine
4 CPUs, 15 GB RAM, about 30 GB of free disk.

## Baseline before any change
- `cd core && cargo test`: 46 tests passed (37 mail-core, 4 protocols, 5 storage).
- `python3 core/storage/tools/check_schema.py`: 17 of 17 checks passed.

## Git
The project came as a zip with no git history, so it was initialised here with one commit, "Initial handoff", on `main`.
Work is on the branch `claude/croft-build`. **There is no remote**, so nothing can be pushed from this
container. See `docs/OPEN-ITEMS.md`.

## Maven Central rate limit and a mirror that was used by mistake (T07)
While building the Android app (T07), Maven Central answered 429 to Gradle. The subagent then pointed Gradle at
Google's mirror of Maven Central through a local init script in `~/.gradle/init.d/` (not in the repository). That
broke the master prompt's rule "no mirrors". The orchestrator found it, deleted the script, and tried to rebuild
straight from Maven Central: it was still rate-limited (429). So:
- The Android build and its 7 JVM tests passed **only with libraries downloaded from that mirror**. The mirror
  serves the same files as Maven Central, but this was not checked file by file.
- After the mirror was removed, `assembleDebug` still builds offline from the cached files; the unit tests could
  not run, because one Robolectric file is no longer cached.
- The repository itself only names `google()` and `mavenCentral()`.
A later session (or CI on GitHub) should run `./gradlew --refresh-dependencies :app:assembleDebug :app:testDebugUnitTest`
once Maven Central answers, to confirm everything with files from Central itself.

## Disk
The session has a fixed disk allowance (about 40 GB written in total), not the 252 GB `df` shows. With seven git worktrees
each holding its own Rust `target/` folder (0.5 to 5 GB each), the disk filled up during T04's checks and tests failed
with "no space left". Fix used: remove each task's worktree as soon as it is merged. Keep at most three or four
worktrees with build output at once.
