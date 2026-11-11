// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import com.example.croft.mail.onboarding.FirstSyncSource
import com.example.croft.mail.onboarding.FirstSyncStatus
import com.example.croft.mail.onboarding.OnboardingBackend
import com.example.croft.mail.MailSession
import com.example.croft.mail.FakeMessageRepository
import com.example.croft.mail.FakeData
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.DiscoveryObserver
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/** The made-up fingerprint of the made-up self-signed server of these tests. */
const val FAKE_FINGERPRINT = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89"

/** Settings as a lookup would find them, for a made-up address. */
fun fakeFound(email: String = "sam@example.org", provider: KnownProvider? = null, host: String = "mail.example.org") = FfiDiscovered(
    email = email,
    imap = ServerSettings(host, 993u, SecurityMode.TLS, email),
    smtp = ServerSettings(host, 465u, SecurityMode.TLS, email),
    jmap = null, source = DiscoverySource.AUTOCONFIG, provider = provider, authHint = AuthKind.PASSWORD,
)

/**
 * A scripted [OnboardingBackend]: each call runs the lambda the test set (and records what it was given), so a test decides what the
 * core would say without a network. [discoverGate] and [verifyGate] hold the call until the test completes them, to look at a screen
 * while it works. Passwords are recorded as text only to check them: made up, never real.
 */
class FakeBackend : OnboardingBackend {
    var onDiscover: suspend (String, DiscoveryObserver) -> FfiDiscovered = { _, _ -> fakeFound() }
    var onVerify: suspend (FfiDiscovered, String, String?) -> Unit = { _, _, _ -> }
    var onAdd: suspend (FfiDiscovered, String?, String) -> Long = { _, _, _ -> 7L }

    var discoverGate: CompletableDeferred<Unit>? = null
    var verifyGate: CompletableDeferred<Unit>? = null

    /** The settings the last `verify` was given. */
    @Volatile var lastVerified: FfiDiscovered? = null

    val discovers = CopyOnWriteArrayList<String>()
    val verifies = CopyOnWriteArrayList<Pair<String?, String>>()
    val adds = CopyOnWriteArrayList<Pair<String?, String>>()

    override suspend fun discover(email: String, observer: DiscoveryObserver): FfiDiscovered {
        discovers += email
        val found = onDiscover(email, observer)
        discoverGate?.await()
        return found
    }

    /** The certificate the fake says the check was made against; null means the one the flow passed (what a server with no stored pin does). */
    @Volatile var verifiedAgainst: String? = null

    override suspend fun verify(discovered: FfiDiscovered, password: ByteArray, pinnedFingerprint: String?): String? {
        val text = String(password)
        verifies += pinnedFingerprint to text
        lastVerified = discovered
        verifyGate?.await()
        onVerify(discovered, text, pinnedFingerprint)
        return verifiedAgainst ?: pinnedFingerprint
    }

    /** Whether each add was given `acceptChangedCertificate`, in order. */
    val acceptFlags = CopyOnWriteArrayList<Boolean>()

    override suspend fun addAccount(
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        acceptChangedCertificate: Boolean,
        password: ByteArray,
    ): Long {
        val text = String(password)
        acceptFlags += acceptChangedCertificate
        adds += pinnedFingerprint to text
        return onAdd(discovered, pinnedFingerprint, text)
    }
}

/** A first sync that reports what the test set. */
class FakeFirstSync(var status: FirstSyncStatus = FirstSyncStatus(0, done = false, failed = false)) : FirstSyncSource {
    override suspend fun status(accountId: Long): FirstSyncStatus = status
}

/** A session over the fake repository, for the environment's `openSession`. */
fun fakeSession(): MailSession {
    val now = 1_700_000_000_000L
    return MailSession(FakeMessageRepository(FakeData.rows(5, now), { now }, FakeData.accounts(now)), usesFakeData = false)
}
