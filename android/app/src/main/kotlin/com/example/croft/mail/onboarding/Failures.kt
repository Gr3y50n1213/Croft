// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.DiscoverException
import uniffi.croft_ffi.VerifyFailure
import uniffi.croft_ffi.discoverErrorMessageKey
import uniffi.croft_ffi.verifyFailureMessageKey

/**
 * What went wrong while adding an account, as the screens show it. It carries the **host** the person typed or the lookup found
 * (the copy names it: "Couldn't connect to mail.example.org"), never a password and never a raw error text. [key] is the
 * translation key: the core's own key for a lookup or sign-in error (`discover.offline`, `verify.wrong_password`, ...), and
 * the app's for the few that only the app can hit (`add.*`, `app.*`). The text for each key is `fail_<key with . as _>` in
 * `strings_onboarding.xml`; `ui/onboarding/FailureCopy.kt` maps keys to strings, and a test walks every key the core can return.
 */
sealed class OnboardingFailure(val key: String, val host: String?) {
    /** Finding the settings failed ([DiscoverException]). */
    class Discover(val error: DiscoverException, host: String?) : OnboardingFailure(discoverErrorMessageKey(error), host)

    /** Signing in or testing the connection failed ([VerifyFailure]). */
    class Verify(val failure: VerifyFailure, host: String?) : OnboardingFailure(verifyFailureMessageKey(failure), host)

    /** Adding the account to the core failed after the sign-in had worked ([CoreException]). */
    class Add(val error: CoreException, host: String?) : OnboardingFailure(addKey(error), host)

    /** The secure storage (Keystore) or the core could not be opened, or the password could not be kept. Nothing was added. */
    class Storage : OnboardingFailure(KEY_STORAGE, null)

    /** Something the app did not expect; no text is kept. */
    class Unexpected : OnboardingFailure(KEY_UNEXPECTED, null)

    /** True for the two failures that mean "the password is wrong" and are shown under the password field. */
    val isPasswordProblem: Boolean
        get() = this is Verify && (failure is VerifyFailure.WrongPassword || failure is VerifyFailure.NeedsAppPasswordOrOAuth)

    companion object {
        const val KEY_STORAGE = "app.storage_unavailable"
        const val KEY_UNEXPECTED = "app.unexpected"

        /** The app's key for a core error from `add_account`. Several errors share copy: the person can do nothing different. */
        fun addKey(e: CoreException): String = when (e) {
            is CoreException.BadSettings, is CoreException.BadFingerprint -> "add.bad_settings"
            is CoreException.UnsupportedServer -> "add.unsupported_server"
            is CoreException.NoCredential -> "add.no_credential"
            is CoreException.SchemaTooNew -> "add.schema_too_new"
            else -> "add.internal"
        }

        /** Every key the app itself defines (the core's come from its own key functions). */
        val appKeys: List<String> = listOf(
            "add.bad_settings", "add.unsupported_server", "add.no_credential", "add.schema_too_new", "add.internal",
            KEY_STORAGE, KEY_UNEXPECTED,
        )
    }
}
