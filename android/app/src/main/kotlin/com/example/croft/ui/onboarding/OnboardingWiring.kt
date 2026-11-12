// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import android.content.Context
import com.example.croft.mail.MailRuntime
import com.example.croft.mail.MailSettingsStore
import com.example.croft.mail.onboarding.CoreFirstSync
import com.example.croft.mail.onboarding.CoreOAuthBackend
import com.example.croft.mail.onboarding.CoreOnboardingBackend
import com.example.croft.mail.onboarding.FirstSyncSource
import com.example.croft.mail.onboarding.FirstSyncStatus
import com.example.croft.mail.onboarding.OnboardingBackend
import com.example.croft.mail.onboarding.StorageUnavailableException
import com.example.croft.secrets.SecretBoxException
import java.io.IOException
import kotlinx.coroutines.CancellationException
import uniffi.croft_ffi.DiscoveryObserver
import uniffi.croft_ffi.FfiDiscovered

/**
 * The sign-in flow over the app's real parts: the core and the secret store of [MailRuntime] (opened on first use, off the main
 * thread), `AccountSecrets.addPasswordAccount` for adding an account (the core first, then the password into `SecretStore`; the
 * account is removed again if the store fails), and a new Inbox session once an account exists.
 */
fun appOnboardingEnvironment(context: Context, settings: MailSettingsStore = MailSettingsStore.shared(context)): OnboardingEnvironment {
    val app = context.applicationContext
    return OnboardingEnvironment(
        backend = AppOnboardingBackend(app),
        firstSync = AppFirstSync(app),
        openSession = { MailRuntime.reloadSession(app, settings) },
        oauth = CoreOAuthBackend { MailRuntime.openedCore(app).core },
    )
}

/** Opens the core when the first call needs it. If it cannot be opened, every call fails with [StorageUnavailableException] (nothing is added). */
private class AppOnboardingBackend(private val context: Context) : OnboardingBackend {
    private suspend fun real(): OnboardingBackend {
        val open = try {
            MailRuntime.openedCore(context)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw StorageUnavailableException()
        } catch (e: UnsatisfiedLinkError) {
            throw StorageUnavailableException()
        }
        return CoreOnboardingBackend(
            open.core,
            adder = { discovered, name, pin, confirmed, password ->
                try {
                    open.accountSecrets.addPasswordAccount(open.core, discovered, name, pin, confirmed, password)
                } catch (e: SecretBoxException) {
                    throw StorageUnavailableException()
                } catch (e: IOException) {
                    throw StorageUnavailableException()
                }
            },
            oauthAdder = { discovered, name, secret ->
                try {
                    open.accountSecrets.addOAuthAccount(open.core, discovered, name, secret)
                } catch (e: SecretBoxException) {
                    throw StorageUnavailableException()
                } catch (e: IOException) {
                    throw StorageUnavailableException()
                }
            },
        )
    }

    override suspend fun discover(email: String, observer: DiscoveryObserver): FfiDiscovered = real().discover(email, observer)

    override suspend fun verify(discovered: FfiDiscovered, password: ByteArray, pinnedFingerprint: String?): String? =
        real().verify(discovered, password, pinnedFingerprint)

    override suspend fun addAccount(
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        confirmedFingerprint: String?,
        password: ByteArray,
    ): Long = real().addAccount(discovered, displayName, pinnedFingerprint, confirmedFingerprint, password)

    override suspend fun verifyOAuth(discovered: FfiDiscovered, secret: ByteArray, pinnedFingerprint: String?): String? =
        real().verifyOAuth(discovered, secret, pinnedFingerprint)

    override suspend fun addOAuthAccount(discovered: FfiDiscovered, displayName: String, secret: ByteArray): Long =
        real().addOAuthAccount(discovered, displayName, secret)

    override fun toString(): String = "[redacted]"
}

/** Reads the first sync's progress from the core. */
private class AppFirstSync(private val context: Context) : FirstSyncSource {
    override suspend fun status(accountId: Long): FirstSyncStatus {
        val open = MailRuntime.core ?: return FirstSyncStatus(0, done = false, failed = false)
        return CoreFirstSync(open.core).status(accountId)
    }
}
