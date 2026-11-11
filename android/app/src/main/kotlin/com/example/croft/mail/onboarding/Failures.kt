// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.DiscoverException
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.VerifyFailure

/**
 * What went wrong while adding an account, as the screens show it. It carries the **host** the person typed or the lookup found
 * (the copy names it: "Couldn't connect to mail.example.org"), never a password and never a raw error text. [key] is the
 * translation key: the core's own key for a lookup or sign-in error (`discover.offline`, `verify.wrong_password`, ...), and
 * the app's for the few that only the app can hit (`add.*`, `app.*`). The core's keys are written out here (an exhaustive `when`, so a
 * new variant does not compile) instead of asked from the core, which keeps the screens free of native calls; a test compares every
 * one with the core's own key functions (`discoverErrorMessageKey`, `verifyFailureMessageKey`). The text for each key is `fail_<key with . as _>` in
 * `strings_onboarding.xml`; `ui/onboarding/FailureCopy.kt` maps keys to strings, and a test walks every key the core can return.
 */
sealed class OnboardingFailure(val key: String, val host: String?) {
    /** Finding the settings failed ([DiscoverException]). */
    class Discover(val error: DiscoverException, host: String?) : OnboardingFailure(discoverKey(error), host)

    /** Signing in or testing the connection failed ([VerifyFailure]). */
    class Verify(val failure: VerifyFailure, host: String?) : OnboardingFailure(verifyKey(failure), host)

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
        /** The core's key for a lookup error (`discover_error_message_key`). */
        fun discoverKey(e: DiscoverException): String = when (e) {
            is DiscoverException.InvalidAddress -> "discover.invalid_address"
            is DiscoverException.Offline -> "discover.offline"
            is DiscoverException.NothingFound -> "discover.nothing_found"
            is DiscoverException.ResolverUnavailable -> "discover.resolver_unavailable"
            is DiscoverException.Internal -> "discover.internal"
        }

        /** The core's key for a sign-in error (`verify_failure_message_key`). */
        fun verifyKey(f: VerifyFailure): String = when (f) {
            is VerifyFailure.WrongPassword -> "verify.wrong_password"
            is VerifyFailure.NeedsAppPasswordOrOAuth -> when (f.provider) {
                KnownProvider.GMAIL -> "verify.needs_app_password.gmail"
                KnownProvider.OUTLOOK -> "verify.needs_app_password.outlook"
                KnownProvider.I_CLOUD -> "verify.needs_app_password.icloud"
                KnownProvider.FASTMAIL -> "verify.needs_app_password.fastmail"
            }
            is VerifyFailure.UntrustedCertificate -> "verify.untrusted_certificate"
            is VerifyFailure.CertificateChanged -> "verify.certificate_changed"
            is VerifyFailure.ConnectionIntercepted -> "verify.connection_intercepted"
            is VerifyFailure.CertificateInvalid -> "verify.certificate_invalid"
            is VerifyFailure.CannotConnect -> "verify.cannot_connect"
            is VerifyFailure.NoSecureConnection -> "verify.no_secure_connection"
            is VerifyFailure.ServerProblem -> "verify.server_problem"
            is VerifyFailure.JmapOnly -> "verify.jmap_only"
            is VerifyFailure.BadSettings -> "verify.bad_settings"
            is VerifyFailure.Internal -> "verify.internal"
        }

        const val KEY_STORAGE = "app.storage_unavailable"
        const val KEY_UNEXPECTED = "app.unexpected"

        /** The app's key for a core error from `add_account`. Several errors share copy: the person can do nothing different. */
        fun addKey(e: CoreException): String = when (e) {
            is CoreException.BadSettings, is CoreException.BadFingerprint -> "add.bad_settings"
            is CoreException.UnsupportedServer -> "add.unsupported_server"
            is CoreException.NoCredential -> "add.no_credential"
            is CoreException.SchemaTooNew -> "add.schema_too_new"
            is CoreException.AccountExists -> "add.already_added"
            else -> "add.internal"
        }

        /** Every key the app itself defines (the core's come from its own key functions). */
        val appKeys: List<String> = listOf(
            "add.bad_settings", "add.unsupported_server", "add.no_credential", "add.schema_too_new", "add.already_added",
            "add.internal", KEY_STORAGE, KEY_UNEXPECTED,
        )
    }
}
