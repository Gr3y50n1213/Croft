#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Fails when the release merged manifest has a permission or an exported component that is not on the allowlist.

Usage: check-release-manifest.py <AndroidManifest.xml> [allowlist.json]
Also fails on android:debuggable="true". The allowlist defaults to manifest-allowlist.json next to this script.
"""
import json
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

A = "{http://schemas.android.com/apk/res/android}"
COMPONENTS = ("activity", "activity-alias", "service", "receiver", "provider")


def check(manifest_path, allowlist_path):
    allow = json.loads(Path(allowlist_path).read_text())
    root = ET.parse(manifest_path).getroot()
    app_id = root.get("package")
    if not app_id:
        return ["manifest has no package attribute"]
    sub = lambda s: s.replace("${applicationId}", app_id)
    allowed_perms = {sub(k) for k in allow["uses-permission"]}
    allowed_decl = {sub(k) for k in allow.get("permission", {})}
    allowed_exp = {sub(k): v for k, v in allow["exported"].items()}
    errors = []

    for el in root.iter():
        if el.tag in ("uses-permission", "uses-permission-sdk-23"):
            name = el.get(A + "name")
            if name not in allowed_perms:
                errors.append(f"permission not on the allowlist: {name}")
        elif el.tag == "permission":
            name = el.get(A + "name")
            if name not in allowed_decl:
                errors.append(f"permission declaration not on the allowlist: {name}")
            elif el.get(A + "protectionLevel") != "signature":
                errors.append(f"permission declaration {name} is not protectionLevel=signature")

    app = root.find("application")
    if app is not None:
        if app.get(A + "debuggable") == "true":
            errors.append('android:debuggable="true" in the release manifest')
        for el in app:
            if el.tag not in COMPONENTS:
                continue
            name = el.get(A + "name") or ""
            if name.startswith("."):
                name = app_id + name
            exported = el.get(A + "exported")
            # Without the attribute a component is exported when it has an intent filter (older targetSdk rule).
            is_exported = exported == "true" or (exported is None and el.find("intent-filter") is not None)
            if not is_exported:
                continue
            key = f"{el.tag}:{name}"
            entry = allowed_exp.get(key)
            if entry is None:
                errors.append(f"exported component not on the allowlist: {key}")
            elif entry.get("permission") != el.get(A + "permission"):
                errors.append(
                    f"exported component {key} is guarded by {el.get(A + 'permission')!r}, allowlist says {entry.get('permission')!r}"
                )
    return errors


def main(argv):
    if not 2 <= len(argv) <= 3:
        print(__doc__, file=sys.stderr)
        return 2
    allowlist = argv[2] if len(argv) == 3 else Path(__file__).with_name("manifest-allowlist.json")
    errors = check(argv[1], allowlist)
    for e in errors:
        print("FAIL", e)
    if not errors:
        print(f"ok   {argv[1]}: permissions and exported components match the allowlist")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
