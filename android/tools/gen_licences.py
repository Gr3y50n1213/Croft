#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Writes the list S60 "Open-source licences" shows: android/app/src/main/assets/licenses/open-source-licences.json.

What goes in (everything the shipped app contains that is not Croft's own code):
  * Rust crates: the normal-dependency closure of croft-ffi for Android, from `cargo metadata --locked --offline`
    (name, version, SPDX licence expression as the crate declares it).
  * Kotlin and Java libraries: each store build's release runtime classpath (`./gradlew --offline :app:dependencies
    --configuration <store>ReleaseRuntimeClasspath`), with each library's licence read from its POM in Gradle's cache. What every
    store has goes in the main list; what only Play has (Firebase) goes in `app/src/play/assets/licenses/store-licences.json`, what
    only F-Droid and GitHub have (UnifiedPush) in `app/src/unifiedpush/assets/licenses/store-licences.json` (T34d). The F-Droid and
    GitHub classpaths must be the same.
  * What the crates carry without a Cargo entry (OpenSSL, SQLCipher, SQLite) and the two fonts.

The result is checked in. `inputs` records the SHA-256 of core/Cargo.lock and android/gradle/libs.versions.toml; the test
`LicenceListTest` (app) and `test_gen_licences.py` (CI, android-release job) fail when they changed without a new list.
Run it again after a dependency change:

    python3 android/tools/gen_licences.py            # from the repository root; needs cargo and a warm Gradle cache

`--check` only compares the recorded hashes with the files (no cargo, no Gradle): that is what CI runs.
"""
import hashlib
import json
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "android/app/src/main/assets/licenses/open-source-licences.json"
STORE_OUT = {
    "play": ROOT / "android/app/src/play/assets/licenses/store-licences.json",
    "fdroid": ROOT / "android/app/src/unifiedpush/assets/licenses/store-licences.json",
}
INPUT_FILES = {"cargoLockSha256": ROOT / "core/Cargo.lock", "libsCatalogSha256": ROOT / "android/gradle/libs.versions.toml"}

# Licences of things that are compiled in without a crate of their own that says so, and the two bundled fonts.
EXTRAS = [
    {"name": "OpenSSL (built from source by openssl-src)", "version": "3.x", "licence": "Apache-2.0", "origin": "Native"},
    {"name": "SQLCipher (bundled by sqlcipher-src)", "version": "4.x", "licence": "BSD-3-Clause", "origin": "Native"},
    {"name": "SQLite (bundled by sqlcipher-src)", "version": "3.x", "licence": "Public domain", "origin": "Native"},
    {"name": "Inter (font)", "version": "", "licence": "OFL-1.1", "origin": "Font"},
    {"name": "Atkinson Hyperlegible (font)", "version": "", "licence": "OFL-1.1", "origin": "Font"},
]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def current_inputs() -> dict:
    return {key: sha256(path) for key, path in INPUT_FILES.items()}


# What the release library is built with: every ABI of `abis` in app/build.gradle.kts, and `rustFeatures` there (SQLCipher with OpenSSL built
# from source, so openssl-sys and openssl-src are in the list). `test_gen_licences.py` checks these two against the Gradle file.
ANDROID_TARGETS = ["aarch64-linux-android", "x86_64-linux-android"]
RUST_FEATURES = "sqlcipher-vendored-openssl"


def rust_entries() -> list:
    """The normal-dependency closure of croft-ffi for each Android target, with the release features, joined."""
    seen_all = {}
    for target in ANDROID_TARGETS:
        meta = json.loads(subprocess.run(
            ["cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", target,
             "--features", f"croft-ffi/{RUST_FEATURES}"],
            cwd=ROOT / "core", check=True, capture_output=True, text=True,
        ).stdout)
        packages = {p["id"]: p for p in meta["packages"]}
        nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
        root = next(p["id"] for p in meta["packages"] if p["name"] == "croft-ffi")
        seen, todo = set(), [root]
        while todo:
            pid = todo.pop()
            if pid in seen:
                continue
            seen.add(pid)
            for dep in nodes[pid]["deps"]:
                # Normal dependencies and build dependencies: a build dependency (openssl-src) puts its code into the library.
                if any(kind["kind"] in (None, "build") for kind in dep["dep_kinds"]):
                    todo.append(dep["pkg"])
        for pid in seen:
            seen_all[pid] = packages[pid]
    out = []
    for p in seen_all.values():
        if p["name"].startswith("croft-"):
            continue  # Croft's own crates (MPL-2.0)
        licence = p.get("license") or "see the crate"
        out.append({"name": p["name"], "version": p["version"], "licence": licence.replace("/", " OR "), "origin": "Rust"})
    return out


# Libraries whose own POM names no licence (it is on a parent POM, which the cache may not hold). Checked by hand.
POM_WITHOUT_LICENCE = {"com.google.guava:listenablefuture": "Apache-2.0"}


def pom_licence(group: str, artifact: str, version: str, cache: Path) -> str:
    base = cache / group / artifact / version
    for pom in sorted(base.glob("*/*.pom")):
        try:
            root = ET.parse(pom).getroot()
        except ET.ParseError:
            continue
        ns = {"m": root.tag.split("}")[0].strip("{")} if root.tag.startswith("{") else {}
        names = [e.text.strip() for e in root.findall(".//m:licenses/m:license/m:name", ns) if e.text] if ns else \
            [e.text.strip() for e in root.findall(".//licenses/license/name") if e.text]
        if names:
            return " OR ".join(dict.fromkeys(spdx(n) for n in names))
    return ""


def spdx(name: str) -> str:
    n = name.lower()
    if "apache" in n:
        return "Apache-2.0"
    if "lgpl" in n or "lesser general public" in n:
        return "LGPL-2.1-or-later"
    if n.startswith("mit"):
        return "MIT"
    # BSD: only from an exact SPDX id or an exact, unambiguous name; anything else keeps the name the library gave (never guessed).
    exact = {"bsd-3-clause": "BSD-3-Clause", "bsd 3-clause": "BSD-3-Clause", "bsd-2-clause": "BSD-2-Clause", "bsd 2-clause": "BSD-2-Clause",
             "the 3-clause bsd license": "BSD-3-Clause", "the bsd 3-clause license": "BSD-3-Clause", "new bsd license": "BSD-3-Clause"}
    if n.strip() in exact:
        return exact[n.strip()]
    return name


def kotlin_entries(deps_report: str) -> list:
    cache = Path(os.environ.get("GRADLE_USER_HOME", Path.home() / ".gradle")) / "caches/modules-2/files-2.1"
    found = {}
    for line in deps_report.splitlines():
        m = re.search(r"[+\\]--- ([\w.\-]+):([\w.\-]+):([\w.\-]+)(?: -> ([\w.\-]+))?", line)
        if not m:
            continue
        group, artifact, declared, resolved = m.groups()
        version = resolved or declared
        # One entry per library: the "-android" and "-jvm" variants and the BOMs are the same project.
        base = re.sub(r"-(android|jvm|iosarm64|linuxx64)$", "", artifact)
        if base.endswith("-bom") or base == "kotlin-stdlib-common":
            continue
        found[(group, base)] = (version, artifact)
    out = []
    for (group, base), (version, artifact) in sorted(found.items()):
        licence = pom_licence(group, artifact, version, cache) or pom_licence(group, base, version, cache)
        licence = licence or POM_WITHOUT_LICENCE.get(f"{group}:{base}", "")
        if not licence:
            raise SystemExit(f"no licence found in the Gradle cache for {group}:{artifact}:{version}")
        out.append({"name": f"{group}:{base}", "version": version, "licence": licence, "origin": "Android"})
    return out


def main(argv: list) -> int:
    if argv[1:] == ["--check"]:
        recorded = json.loads(OUT.read_text()).get("inputs", {})
        if recorded != current_inputs():
            print("open-source-licences.json is out of date: run python3 android/tools/gen_licences.py", file=sys.stderr)
            return 1
        return 0
    def classpath(store: str) -> list:
        report = subprocess.run(
            ["./gradlew", "--offline", "-q", ":app:dependencies", "--configuration", f"{store}ReleaseRuntimeClasspath"],
            cwd=ROOT / "android", check=True, capture_output=True, text=True,
        ).stdout
        return kotlin_entries(report)

    by_store = {store: classpath(store) for store in ("play", "fdroid", "github")}
    key = lambda e: (e["name"], e["version"])
    if sorted(map(key, by_store["fdroid"])) != sorted(map(key, by_store["github"])):
        raise SystemExit("the fdroid and github classpaths differ: they share one licence list")
    common = {key(e) for e in by_store["play"]} & {key(e) for e in by_store["fdroid"]}
    order = lambda e: (e["origin"], e["name"].lower())
    entries = sorted(rust_entries() + [e for e in by_store["play"] if key(e) in common], key=order) + EXTRAS
    write(OUT, {"version": 1, "inputs": current_inputs(), "entries": entries})
    for store, path in STORE_OUT.items():
        write(path, {"version": 1, "entries": sorted((e for e in by_store[store] if key(e) not in common), key=order)})
    return 0


def write(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n")
    print(f"wrote {len(data['entries'])} entries to {path.relative_to(ROOT)}")


if __name__ == "__main__":
    sys.exit(main(sys.argv))
