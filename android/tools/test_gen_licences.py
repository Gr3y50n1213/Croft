# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""The checked-in licence list (S60) must be there, non-empty, and made from the current dependency files."""
import json
import unittest

import gen_licences


class LicenceListTest(unittest.TestCase):
    def setUp(self):
        self.data = json.loads(gen_licences.OUT.read_text())

    def test_the_list_names_rust_and_android_dependencies(self):
        origins = {e["origin"] for e in self.data["entries"]}
        self.assertTrue({"Rust", "Android", "Font"} <= origins)
        self.assertGreater(len(self.data["entries"]), 100)

    def test_every_entry_has_a_name_and_a_licence(self):
        for e in self.data["entries"]:
            self.assertTrue(e["name"].strip(), e)
            self.assertTrue(e["licence"].strip(), e)

    def test_it_was_made_from_the_current_lock_file_and_catalog(self):
        self.assertEqual(self.data["inputs"], gen_licences.current_inputs(),
                         "dependencies changed: run python3 android/tools/gen_licences.py")


class ReleaseBuildMatchTest(unittest.TestCase):
    """The list is made for what the release build compiles: its ABIs and its Rust features (app/build.gradle.kts)."""

    def setUp(self):
        self.gradle = (gen_licences.ROOT / "android/app/build.gradle.kts").read_text()

    def test_every_abi_of_the_release_build_is_covered(self):
        import re
        abis = re.search(r'val abis = listOf\(([^)]*)\)', self.gradle).group(1)
        names = re.findall(r'"([^"]+)"', abis)
        triple = {"arm64-v8a": "aarch64-linux-android", "x86_64": "x86_64-linux-android", "armeabi-v7a": "armv7-linux-androideabi", "x86": "i686-linux-android"}
        self.assertEqual(sorted(triple[a] for a in names), sorted(gen_licences.ANDROID_TARGETS))

    def test_the_release_rust_features_are_the_ones_the_list_was_made_with(self):
        import re
        features = re.search(r'val rustFeatures = "([^"]+)"', self.gradle).group(1)
        self.assertEqual(features, gen_licences.RUST_FEATURES)

    def test_the_openssl_crates_are_listed(self):
        names = {e["name"] for e in json.loads(gen_licences.OUT.read_text())["entries"]}
        self.assertTrue({"openssl-sys", "openssl-src", "sqlcipher-src"} <= names)



class ParentPomTest(unittest.TestCase):
    def test_a_licence_named_only_in_the_parent_pom_is_found(self):
        import tempfile
        from pathlib import Path as P
        with tempfile.TemporaryDirectory() as d:
            c = P(d)
            (c / "x.y/child/1.0/h").mkdir(parents=True)
            (c / "x.y/parent/1.0/h").mkdir(parents=True)
            ns = 'xmlns="http://maven.apache.org/POM/4.0.0"'
            (c / "x.y/child/1.0/h/child-1.0.pom").write_text(
                f"<project {ns}><parent><groupId>x.y</groupId><artifactId>parent</artifactId><version>1.0</version></parent></project>")
            (c / "x.y/parent/1.0/h/parent-1.0.pom").write_text(
                f"<project {ns}><licenses><license><name>BSD-3-Clause</name></license></licenses></project>")
            self.assertEqual("BSD-3-Clause", gen_licences.pom_licence("x.y", "child", "1.0", c))
            self.assertEqual("", gen_licences.pom_licence("x.y", "missing", "1.0", c))

if __name__ == "__main__":
    unittest.main()
