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


if __name__ == "__main__":
    unittest.main()
