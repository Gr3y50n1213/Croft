// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.AccountStatus
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.DiscoveryObserver
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered

/** The secure storage (or the core) could not be opened, or the password could not be kept: no account was added. */
class StorageUnavailableException : Exception()

/**
 * The person signed in again for an account that already exists, and the new password could not be kept. The core's update of the
 * account was undone (`Core.rollbackAccountUpdate`), so the account is as it was, with its old password. Nothing was added.
 */
class SignInNotSavedException : Exception()

/**
 * What the sign-in flow needs from the Rust core (T10's `discover`, `verify` and `add_account`) and from the secret store. The
 * flow talks only to this interface, so a screen test can script it and the JVM tests can run it over the real core.
 *
 * **Passwords** cross as UTF-8 `ByteArray`. A call never keeps the array and never wipes it: the caller ([OnboardingFlow]) owns it
 * and wipes it. Each call makes its own copy for the core's credential object and wipes that copy.
 *
 * Failures are the typed ones of the core: `discover` throws `DiscoverException`, `verify` throws `VerifyFailure`, `addAccount`
 * throws `CoreException`, and [StorageUnavailableException] when the password could not be stored (the account is then not added).
 */
interface OnboardingBackend {
    /** Looks up the settings for [email], reporting each probe to [observer]. Cancelling the coroutine stops the probes. */
    suspend fun discover(email: String, observer: DiscoveryObserver): FfiDiscovered

    /**
     * Signs in with [discovered] and [password] to check them; [pinnedFingerprint] is a certificate the person trusted. Returns the
     * fingerprint the connection was checked against (the one passed, or the stored one of an account with this address and server),
     * which is what [addAccount] must be given as its [pinnedFingerprint]; null when there is none.
     */
    suspend fun verify(discovered: FfiDiscovered, password: ByteArray, pinnedFingerprint: String?): String?

    /**
     * Adds the verified account and keeps [password] in the secret store under the account's id, which is returned. For an address
     * that is already added with the same server the id is the existing account's and the new password replaces the old one.
     * [acceptChangedCertificate] is true **only** after the person confirmed the changed-certificate warning in this attempt: without
     * it the core refuses to replace or drop a pin it holds (`CoreException.CertificateChanged`).
     */
    suspend fun addAccount(
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        acceptChangedCertificate: Boolean,
        password: ByteArray,
    ): Long
}

/**
 * Where the password is stored once the core has added the account: `(discovered, displayName, pinnedFingerprint, acceptChangedCertificate, password)` to the
 * account's id. In the app this is `AccountSecrets.addPasswordAccount` (core call first, then the secret store, and a new account
 * is removed again if the store fails; an account that was only signed in again is kept, with the new password replacing the old);
 * the JVM tests give a plain call to the core and store the password themselves.
 */
typealias AccountAdder = suspend (FfiDiscovered, String, String?, Boolean, ByteArray) -> Long

/** [OnboardingBackend] over the real [Core]. */
class CoreOnboardingBackend(private val core: Core, private val adder: AccountAdder) : OnboardingBackend {
    override suspend fun discover(email: String, observer: DiscoveryObserver): FfiDiscovered = core.discover(email, observer)

    override suspend fun verify(discovered: FfiDiscovered, password: ByteArray, pinnedFingerprint: String?): String? {
        val copy = password.copyOf()
        try {
            val credential = FfiCredential.password(copy)
            try {
                return core.verify(discovered, credential, pinnedFingerprint)
            } finally {
                credential.close()
            }
        } finally {
            copy.fill(0)
        }
    }

    /**
     * Adds the account. **The core decides** what an address that is already added means (`add_account`): the same server is the
     * person signing in again, so the account keeps its id and gets the new settings, pin and password; another server fails with
     * `CoreException.AccountExists` ("This address is already added"), and a trusted certificate is never replaced or dropped without
     * [acceptChangedCertificate] (`CoreException.CertificateChanged`). That is also what makes a second sign-in harmless when the
     * first one did add the account but its screen was gone before it could say so (the add runs to its end even then, see
     * `OnboardingFlow`): the second one finds the account and refreshes it. Nothing here reads the database.
     */
    override suspend fun addAccount(
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        acceptChangedCertificate: Boolean,
        password: ByteArray,
    ): Long = adder(discovered, displayName, pinnedFingerprint, acceptChangedCertificate, password)

    override fun toString(): String = "[redacted]"
}

/** How far the first sync of a new account has come (S09). */
data class FirstSyncStatus(
    /** Messages in the Inbox so far. */
    val inboxMessages: Int,
    /** The first folders have synced: the Inbox can be read. */
    val done: Boolean,
    /** The account stopped (the server refused the sign-in, or it cannot run): the sync will not finish by itself. */
    val failed: Boolean,
)

/** Reads the progress of the first sync of an account. */
interface FirstSyncSource {
    /** The status now. Cheap; S09 asks about twice a second. */
    suspend fun status(accountId: Long): FirstSyncStatus
}

/** [FirstSyncSource] over the core: done when a folder has synced, failed when the account needs a sign-in or cannot run. */
class CoreFirstSync(private val core: Core) : FirstSyncSource {
    override suspend fun status(accountId: Long): FirstSyncStatus = withContext(Dispatchers.IO) {
        val account = core.listAccounts().firstOrNull { it.id == accountId } ?: return@withContext FirstSyncStatus(0, done = false, failed = true)
        val inbox = core.listFolders(accountId).firstOrNull { it.specialUse == "inbox" }?.totalCount?.toInt() ?: 0
        FirstSyncStatus(
            inboxMessages = inbox,
            done = account.lastSyncedMs != null && account.status != AccountStatus.SYNCING,
            failed = account.status == AccountStatus.NEEDS_SIGN_IN || account.status == AccountStatus.ERROR,
        )
    }
}
