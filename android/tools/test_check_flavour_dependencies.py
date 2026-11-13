# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Tests for check-flavour-dependencies.py: run with `python3 -m unittest discover -s tools` from android/."""
import importlib.util
import unittest
from pathlib import Path

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location("cfd", HERE / "check-flavour-dependencies.py")
cfd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cfd)

CLEAN = r"""
fdroidReleaseRuntimeClasspath - Runtime classpath of '/fdroidRelease'.
+--- project :swipe
+--- androidx.compose:compose-bom:2026.09.00
|    +--- androidx.compose.ui:ui:1.12.0 (c)
+--- androidx.core:core-ktx:1.19.1
|    \--- androidx.core:core:1.19.1
+--- com.google.android.material:material:1.14.0
|    \--- com.google.errorprone:error_prone_annotations:2.30.0
\--- net.java.dev.jna:jna:5.19.1
"""


class FlavourDependencies(unittest.TestCase):
    def test_a_clean_report_passes(self):
        self.assertEqual([], cfd.problems(CLEAN))

    def test_open_source_google_libraries_are_allowed(self):
        self.assertEqual([], cfd.problems(CLEAN + "+--- com.google.guava:guava:33.0.0\n"))

    def test_each_forbidden_group_fails_also_deep_in_the_tree(self):
        for group in cfd.FORBIDDEN:
            for prefix in ("+--- ", "|    |    \\--- "):
                text = CLEAN + f"{prefix}{group}:some-lib:1.0 -> 1.1 (*)\n"
                self.assertEqual([f"forbidden Google services library: {group}:some-lib"], cfd.problems(text), (group, prefix))

    def test_a_sub_group_fails_but_a_look_alike_does_not(self):
        self.assertTrue(cfd.problems(CLEAN + "+--- com.google.firebase.crashlytics:x:1\n"))
        self.assertEqual([], cfd.problems(CLEAN + "+--- com.google.firebasex:x:1\n"))
        self.assertEqual([], cfd.problems(CLEAN + "+--- com.google.android.playground:x:1\n"))

    def test_an_empty_or_failed_report_fails(self):
        self.assertTrue(cfd.problems(""))
        self.assertTrue(cfd.problems("+--- project :swipe\n"))
        self.assertTrue(cfd.problems(CLEAN + "+--- org.example:x:1 FAILED\n"))

    def test_main_reports_each_file(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            good = Path(d, "good.txt")
            good.write_text(CLEAN)
            bad = Path(d, "bad.txt")
            bad.write_text(CLEAN + "+--- com.google.firebase:firebase-messaging:24.0.0\n")
            self.assertEqual(0, cfd.main([str(good)]))
            self.assertEqual(1, cfd.main([str(good), str(bad)]))


if __name__ == "__main__":
    unittest.main()
