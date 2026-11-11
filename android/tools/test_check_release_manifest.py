# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Tests for check-release-manifest.py: run with `python3 -m unittest discover -s tools` from android/."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location("crm", HERE / "check-release-manifest.py")
crm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(crm)
ALLOW = HERE / "manifest-allowlist.json"

BASE = """<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.example.croft">
  <permission android:name="com.example.croft.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION" android:protectionLevel="signature" />
  <uses-permission android:name="com.example.croft.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION" />
  %s
  <application>
    <activity android:name="com.example.croft.MainActivity" android:exported="true">
      <intent-filter><action android:name="android.intent.action.MAIN" /></intent-filter>
    </activity>
    <provider android:name="androidx.startup.InitializationProvider" android:authorities="x" android:exported="false" />
    <receiver android:name="androidx.profileinstaller.ProfileInstallReceiver" android:exported="true" android:permission="android.permission.DUMP" />
    %s
  </application>
</manifest>"""


def run(extra_top="", extra_app=""):
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / "AndroidManifest.xml"
        p.write_text(BASE % (extra_top, extra_app))
        return crm.check(p, ALLOW)


class ManifestCheck(unittest.TestCase):
    def test_current_shape_passes(self):
        self.assertEqual(run(), [])

    def test_fake_permission_fails(self):
        errs = run(extra_top='<uses-permission android:name="android.permission.INTERNET" />')
        self.assertEqual(len(errs), 1)
        self.assertIn("android.permission.INTERNET", errs[0])

    def test_fake_exported_receiver_fails(self):
        errs = run(extra_app='<receiver android:name="com.example.croft.Evil" android:exported="true" />')
        self.assertEqual(len(errs), 1)
        self.assertIn("receiver:com.example.croft.Evil", errs[0])

    def test_implicitly_exported_service_fails(self):
        errs = run(extra_app='<service android:name=".S"><intent-filter><action android:name="a" /></intent-filter></service>')
        self.assertEqual(len(errs), 1)

    def test_profile_receiver_must_keep_its_guard(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "m.xml"
            p.write_text((BASE % ("", "")).replace(' android:permission="android.permission.DUMP"', ""))
            self.assertEqual(len(crm.check(p, ALLOW)), 1)

    def test_debuggable_fails(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "m.xml"
            p.write_text((BASE % ("", "")).replace("<application>", '<application android:debuggable="true">'))
            self.assertEqual(len(crm.check(p, ALLOW)), 1)


if __name__ == "__main__":
    unittest.main()
