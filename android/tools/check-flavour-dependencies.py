#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Fails when a build without Google services would ship a Google Play services, Firebase or Play library (T34c).

Usage: check-flavour-dependencies.py <dependencies.txt> [...]
Each file is the output of `./gradlew :app:dependencies --configuration <flavor>ReleaseRuntimeClasspath` for `fdroid` or `github`.
Open-source Google libraries (AndroidX, Material Components, Compose) are allowed: F-Droid accepts them. What is refused is the
proprietary client side of Google's services: Play services, Firebase, the Play Core and billing libraries, ML Kit, the consent SDK
and the transport they share. A file that does not look like a resolved classpath (no AndroidX at all, or a resolution failure)
fails too, so an empty or broken report never passes quietly.
"""
import re
import sys
from pathlib import Path

# Groups (and their sub-groups) that must not reach fdroid or github.
FORBIDDEN = (
    "com.google.android.gms",
    "com.google.firebase",
    "com.google.android.play",
    "com.google.android.datatransport",
    "com.google.android.ump",
    "com.google.mlkit",
    "com.android.billingclient",
    "com.android.installreferrer",
)

# A tree line: "+--- group:name:version", "\--- group:name -> version (*)"; projects ("project :swipe") have no group.
LINE = re.compile(r"^[| ]*[+\\]--- (?P<coord>[^\s]+)")


def coordinates(text: str) -> list[str]:
    """Every `group:name[:version]` in a Gradle dependency report."""
    out = []
    for line in text.splitlines():
        m = LINE.match(line)
        if m and ":" in m.group("coord"):
            out.append(m.group("coord"))
    return out


def forbidden(group: str) -> bool:
    return any(group == g or group.startswith(g + ".") for g in FORBIDDEN)


def problems(text: str) -> list[str]:
    """What is wrong with one report; empty when it passes."""
    if "FAILED" in text or "Could not resolve" in text:
        return ["the report has a resolution failure: fix the build first"]
    coords = coordinates(text)
    if not any(c.startswith("androidx.") for c in coords):
        return ["no AndroidX library in the report: this is not a resolved runtime classpath"]
    found = sorted({c.split(":")[0] + ":" + c.split(":")[1] for c in coords if forbidden(c.split(":")[0])})
    return [f"forbidden Google services library: {c}" for c in found]


def main(paths: list[str]) -> int:
    if not paths:
        print(__doc__)
        return 2
    fail = 0
    for p in paths:
        issues = problems(Path(p).read_text(encoding="utf-8"))
        for i in issues:
            print(f"FAIL {p}: {i}")
        if not issues:
            print(f"ok   {p}: no Google services library")
        fail |= bool(issues)
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
