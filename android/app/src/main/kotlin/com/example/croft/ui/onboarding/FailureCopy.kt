// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import com.example.croft.R
import com.example.croft.mail.onboarding.OnboardingFailure
import uniffi.croft_ffi.ProbeFailureReason

/**
 * The plain-language text of every failure the sign-in flow can show. The keys are the core's own (`discover.*`, `verify.*`, from
 * `core/ffi/src/error.rs`) and the app's (`add.*`, `app.*`); the text of each is in `strings_onboarding.xml` as `fail_<key>` with the dots
 * as underscores, and takes the host as `%1$s`. `FailureCopyTest` walks every key the core can return and fails when one has no
 * text here. No raw code is ever shown: technical lines sit behind a Details link.
 */
object FailureCopy {
    /** The string for a translation key, or null for a key this table does not know (a test fails on that). */
    @StringRes
    fun resFor(key: String): Int? = when (key) {
        "discover.invalid_address" -> R.string.fail_discover_invalid_address
        "discover.offline" -> R.string.fail_discover_offline
        "discover.nothing_found" -> R.string.fail_discover_nothing_found
        "discover.resolver_unavailable" -> R.string.fail_discover_resolver_unavailable
        "discover.internal" -> R.string.fail_discover_internal
        "verify.wrong_password" -> R.string.fail_verify_wrong_password
        "verify.needs_app_password.gmail" -> R.string.fail_verify_needs_app_password_gmail
        "verify.needs_app_password.outlook" -> R.string.fail_verify_needs_app_password_outlook
        "verify.needs_app_password.icloud" -> R.string.fail_verify_needs_app_password_icloud
        "verify.needs_app_password.fastmail" -> R.string.fail_verify_needs_app_password_fastmail
        "verify.untrusted_certificate" -> R.string.fail_verify_untrusted_certificate
        "verify.certificate_changed" -> R.string.fail_verify_certificate_changed
        "verify.connection_intercepted" -> R.string.fail_verify_connection_intercepted
        "verify.certificate_invalid" -> R.string.fail_verify_certificate_invalid
        "verify.cannot_connect" -> R.string.fail_verify_cannot_connect
        "verify.no_secure_connection" -> R.string.fail_verify_no_secure_connection
        "verify.server_problem" -> R.string.fail_verify_server_problem
        "verify.jmap_only" -> R.string.fail_verify_jmap_only
        "verify.bad_settings" -> R.string.fail_verify_bad_settings
        "verify.internal" -> R.string.fail_verify_internal
        "add.bad_settings" -> R.string.fail_add_bad_settings
        "add.unsupported_server" -> R.string.fail_add_unsupported_server
        "add.no_credential" -> R.string.fail_add_no_credential
        "add.schema_too_new" -> R.string.fail_add_schema_too_new
        "add.already_added" -> R.string.fail_add_already_added
        "add.internal" -> R.string.fail_add_internal
        "app.storage_unavailable" -> R.string.fail_app_storage_unavailable
        "app.unexpected" -> R.string.fail_app_unexpected
        else -> null
    }

    /** The name of the string resource for [key] (`fail_verify_wrong_password` for `verify.wrong_password`). */
    fun resourceName(key: String): String = "fail_" + key.replace('.', '_')

    /** The reason a probe found nothing, in words (behind Details). Exhaustive, so a new core reason is a compile error here. */
    @StringRes
    fun probeReasonRes(reason: ProbeFailureReason): Int = when (reason) {
        ProbeFailureReason.NOT_PUBLISHED -> R.string.onb_reason_not_published
        ProbeFailureReason.TIMED_OUT -> R.string.onb_reason_timed_out
        ProbeFailureReason.UNREACHABLE -> R.string.onb_reason_unreachable
        ProbeFailureReason.CERTIFICATE_PROBLEM -> R.string.onb_reason_certificate_problem
        ProbeFailureReason.UNUSABLE -> R.string.onb_reason_unusable
        ProbeFailureReason.TOO_LARGE -> R.string.onb_reason_too_large
        ProbeFailureReason.BAD_REDIRECT -> R.string.onb_reason_bad_redirect
        ProbeFailureReason.ONLY_UNENCRYPTED -> R.string.onb_reason_only_unencrypted
        ProbeFailureReason.INTERNAL -> R.string.onb_reason_internal
    }
}

/** The failure in words, with its host filled in. An unknown key falls back to the unexpected-error text. */
@Composable
fun failureText(failure: OnboardingFailure): String {
    val host = failure.host ?: stringResource(R.string.onb_host_fallback)
    return stringResource(FailureCopy.resFor(failure.key) ?: R.string.fail_app_unexpected, host)
}
