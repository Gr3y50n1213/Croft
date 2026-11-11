# Decisions

One line per choice made without the owner: the decision, the reason, the date. Dependency versions too.

| Date | Decision | Reason |
|---|---|---|
| 2026-11-11 | Repository root is the unzipped `croft/` folder; work happens on branch `claude/croft-build`, `main` holds only "Initial handoff" | Master prompt: never push to the default branch; owner merges |
| 2026-11-11 | Android SDK: compile and target API 36, build-tools 36.0.0, NDK 29.0.14206865 (newest r29, stable line) | `docs/store/READINESS.md` asks for API 36; NDK r28+ builds 16 KB-page-aligned libraries by default |
| 2026-11-11 | Licence texts fetched from https://www.mozilla.org/media/MPL/2.0/index.txt and https://www.gnu.org/licenses/agpl-3.0.txt | The plain-text links named in `LICENSES.md` |
