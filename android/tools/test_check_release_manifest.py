# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Tests for check-release-manifest.py: run with `python3 -m unittest discover -s tools` from android/."""
import importlib.util
import os
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
        errs = run(extra_top='<uses-permission android:name="android.permission.READ_CONTACTS" />')
        self.assertEqual(len(errs), 1)
        self.assertIn("android.permission.READ_CONTACTS", errs[0])

    def test_the_two_network_permissions_of_t16_pass(self):
        top = (
            '<uses-permission android:name="android.permission.INTERNET" />'
            '<uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />'
        )
        self.assertEqual(run(extra_top=top), [])

    def test_the_compose_intent_filters_of_t18d_pass(self):
        manifest = BASE.replace(
            '<intent-filter><action android:name="android.intent.action.MAIN" /></intent-filter>',
            '<intent-filter><action android:name="android.intent.action.MAIN" /></intent-filter>'
            '<intent-filter><action android:name="android.intent.action.VIEW" /><category android:name="android.intent.category.BROWSABLE" />'
            '<data android:scheme="mailto" /></intent-filter>'
            '<intent-filter><action android:name="android.intent.action.SEND" /><data android:mimeType="*/*" /></intent-filter>',
        )
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "AndroidManifest.xml"
            p.write_text(manifest % ("", ""))
            self.assertEqual(crm.check(p, ALLOW), [])

    GRANT = (
        '<provider android:name="androidx.core.content.FileProvider" android:authorities="com.example.croft.attachments" '
        'android:exported="false" android:grantUriPermissions="true">'
        '<meta-data android:name="android.support.FILE_PROVIDER_PATHS" android:resource="@xml/shared_attachment_paths" /></provider>'
    )

    def test_the_attachment_file_provider_of_t18b_passes(self):
        self.assertEqual(run(extra_app=self.GRANT), [])

    def test_an_exported_file_provider_fails(self):
        errs = run(extra_app=self.GRANT.replace('android:exported="false"', 'android:exported="true"'))
        self.assertTrue(any("must have android:exported" in e for e in errs), errs)
        self.assertTrue(any("exported component" in e for e in errs), errs)

    def test_a_provider_with_an_unlisted_authority_fails(self):
        errs = run(extra_app=self.GRANT.replace("com.example.croft.attachments", "com.example.croft.everything"))
        self.assertEqual(len(errs), 1)
        self.assertIn("authority is not on the allowlist", errs[0])

    def test_a_file_provider_with_another_paths_file_fails(self):
        errs = run(extra_app=self.GRANT.replace("@xml/shared_attachment_paths", "@xml/all_paths"))
        self.assertEqual(len(errs), 1)
        self.assertIn("paths file", errs[0])

    def test_a_provider_granting_through_child_elements_is_checked_too(self):
        child = (
            '<provider android:name="com.example.Leaky" android:authorities="com.example.croft.leaky" android:exported="false">'
            '<grant-uri-permission android:pathPrefix="/" /></provider>'
        )
        errs = run(extra_app=child)
        self.assertTrue(any("authority is not on the allowlist" in e for e in errs), errs)

    def test_a_provider_that_does_not_grant_uri_permissions_is_not_checked(self):
        # The startup provider of the base manifest is such a provider and passes.
        self.assertEqual(run(), [])

    def test_a_browsable_web_link_filter_fails(self):
        manifest = BASE.replace(
            '<intent-filter><action android:name="android.intent.action.MAIN" /></intent-filter>',
            '<intent-filter><action android:name="android.intent.action.VIEW" /><category android:name="android.intent.category.BROWSABLE" />'
            '<data android:scheme="https" /></intent-filter>',
        )
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "AndroidManifest.xml"
            p.write_text(manifest % ("", ""))
            errs = crm.check(p, ALLOW)
        self.assertEqual(len(errs), 1)
        self.assertIn("https", errs[0])

    COMPANION = (
        '<activity android:name="com.example.croft.CompanionRedirectActivity" android:exported="true">'
        '<intent-filter><action android:name="android.intent.action.VIEW" />'
        '<category android:name="android.intent.category.DEFAULT" /><category android:name="android.intent.category.BROWSABLE" />'
        '<data android:scheme="%s" /></intent-filter></activity>'
    )

    def test_the_companion_redirect_of_t34b_passes_only_for_the_apps_own_scheme(self):
        self.assertEqual(run(extra_app=self.COMPANION % "com.example.croft"), [])
        errs = run(extra_app=self.COMPANION % "https")
        self.assertEqual(len(errs), 1)
        self.assertIn("scheme not on the allowlist: https", errs[0])
        errs = run(extra_app=self.COMPANION % "com.example.other")
        self.assertEqual(len(errs), 1)

    def test_an_action_nobody_handles_fails(self):
        manifest = BASE.replace(
            '<intent-filter><action android:name="android.intent.action.MAIN" /></intent-filter>',
            '<intent-filter><action android:name="android.intent.action.PICK" /></intent-filter>',
        )
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "AndroidManifest.xml"
            p.write_text(manifest % ("", ""))
            errs = crm.check(p, ALLOW)
        self.assertEqual(len(errs), 1)
        self.assertIn("PICK", errs[0])

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

    def test_the_notification_and_workmanager_entries_of_t20_pass(self):
        top = (
            '<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />'
            '<uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED" />'
            '<uses-permission android:name="android.permission.WAKE_LOCK" />'
        )
        app = (
            '<service android:name="androidx.work.impl.background.systemjob.SystemJobService" android:exported="true"'
            ' android:permission="android.permission.BIND_JOB_SERVICE" />'
            '<receiver android:name="androidx.work.impl.diagnostics.DiagnosticsReceiver" android:exported="true"'
            ' android:permission="android.permission.DUMP" />'
            '<activity-alias android:name=".OpenFromNotification" android:exported="false" android:targetActivity=".MainActivity" />'
            '<receiver android:name=".notify.NotificationActionReceiver" android:exported="false" />'
        )
        self.assertEqual(run(extra_top=top, extra_app=app), [])

    def test_the_job_service_must_keep_its_guard(self):
        errs = run(extra_app='<service android:name="androidx.work.impl.background.systemjob.SystemJobService" android:exported="true" />')
        self.assertEqual(len(errs), 1)

    def test_store_forbidden_permissions_fail_even_if_allowlisted(self):
        import json
        for name in (
            "android.permission.SCHEDULE_EXACT_ALARM",
            "android.permission.USE_EXACT_ALARM",
            "android.permission.FOREGROUND_SERVICE",
            "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
            "android.permission.READ_CONTACTS",
            "android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS",
        ):
            with tempfile.TemporaryDirectory() as d:
                allow = json.loads(ALLOW.read_text())
                allow["uses-permission"][name] = "a mistake"
                a = Path(d) / "allow.json"
                a.write_text(json.dumps(allow))
                p = Path(d) / "m.xml"
                p.write_text(BASE % (f'<uses-permission android:name="{name}" />', ""))
                errs = crm.check(p, a)
                self.assertEqual(len(errs), 1, name)
                self.assertIn("forbidden", errs[0])

    def test_a_foreground_service_type_fails_even_unexported(self):
        errs = run(extra_app='<service android:name="androidx.work.impl.foreground.SystemForegroundService" android:exported="false"'
                   ' android:foregroundServiceType="dataSync" />')
        self.assertEqual(len(errs), 1)
        self.assertIn("foreground service type", errs[0])

    def test_debuggable_fails(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "m.xml"
            p.write_text((BASE % ("", "")).replace("<application>", '<application android:debuggable="true">'))
            self.assertEqual(len(crm.check(p, ALLOW)), 1)

    def run_store(self, store, extra_top="", extra_app=""):
        old = os.environ.get("CROFT_STORE")
        if store is None:
            os.environ.pop("CROFT_STORE", None)
        else:
            os.environ["CROFT_STORE"] = store
        try:
            return run(extra_top, extra_app)
        finally:
            if old is None:
                os.environ.pop("CROFT_STORE", None)
            else:
                os.environ["CROFT_STORE"] = old

    FCM_TOP = '<uses-permission android:name="com.google.android.c2dm.permission.RECEIVE" />'
    FCM_APP = (
        '<receiver android:name="com.google.firebase.iid.FirebaseInstanceIdReceiver" android:exported="true" '
        'android:permission="com.google.android.c2dm.permission.SEND">'
        '<intent-filter><action android:name="com.google.android.c2dm.intent.RECEIVE" /></intent-filter></receiver>'
    )
    UP_APP = (
        '<receiver android:name="org.unifiedpush.android.connector.internal.MessagingReceiverImpl" android:exported="true">'
        '<intent-filter><action android:name="org.unifiedpush.android.connector.MESSAGE" />'
        '<action android:name="org.unifiedpush.android.connector.NEW_ENDPOINT" /></intent-filter></receiver>'
        '<service android:name="org.unifiedpush.android.connector.internal.RaiseToForegroundService" android:exported="true">'
        '<intent-filter><action android:name="org.unifiedpush.android.connector.RAISE_TO_FOREGROUND" /></intent-filter></service>'
    )

    def test_fcm_of_t34d_passes_only_in_the_play_build(self):
        self.assertEqual(self.run_store("play", self.FCM_TOP, self.FCM_APP), [])
        for store in ("fdroid", "github", None):
            errs = self.run_store(store, self.FCM_TOP, self.FCM_APP)
            self.assertEqual(len(errs), 2, (store, errs))

    def test_unifiedpush_of_t34d_passes_only_in_fdroid_and_github(self):
        for store in ("fdroid", "github"):
            self.assertEqual(self.run_store(store, extra_app=self.UP_APP), [], store)
        self.assertEqual(len(self.run_store("play", extra_app=self.UP_APP)), 2)

    def test_the_fcm_receiver_must_keep_its_guard(self):
        unguarded = self.FCM_APP.replace(' android:permission="com.google.android.c2dm.permission.SEND"', "")
        self.assertEqual(len(self.run_store("play", self.FCM_TOP, unguarded)), 1)

    def test_the_unifiedpush_receiver_takes_no_other_action(self):
        extra = self.UP_APP.replace(
            '<action android:name="org.unifiedpush.android.connector.MESSAGE" />',
            '<action android:name="android.intent.action.VIEW" />',
        )
        self.assertEqual(len(self.run_store("fdroid", extra_app=extra)), 1)

    def test_preview_suffix_keeps_the_activity_in_the_namespace(self):
        # The preview build's package is "<namespace>.preview"; its classes and its signature permission differ accordingly.
        import os
        text = BASE.replace('package="com.example.croft"', 'package="com.example.croft.preview"').replace(
            "com.example.croft.DYNAMIC", "com.example.croft.preview.DYNAMIC"
        )
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "m.xml"
            p.write_text(text % ("", ""))
            os.environ["CROFT_NAMESPACE"] = "com.example.croft"
            try:
                self.assertEqual(crm.check(p, ALLOW), [])
            finally:
                del os.environ["CROFT_NAMESPACE"]
            # Without the namespace the activity is not the allowlisted one.
            self.assertEqual(len(crm.check(p, ALLOW)), 1)


if __name__ == "__main__":
    unittest.main()
