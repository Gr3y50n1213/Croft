// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import uniffi.croft_ffi.CredentialProvider
import uniffi.croft_ffi.FfiCredential

/** The accounts that need the person to sign in again, for the screens to observe. */
class SignInNeeded {
    private val state = MutableStateFlow<Set<Long>>(emptySet())

    /** Account ids (local row ids) waiting for a sign-in. */
    val accounts: StateFlow<Set<Long>> get() = state

    /** The core said the account needs a sign-in. */
    fun mark(accountId: Long) {
        state.value = state.value + accountId
    }

    /** The person signed in again (or the account was removed). */
    fun clear(accountId: Long) {
        state.value = state.value - accountId
    }

    override fun toString(): String = "[redacted]"
}

/**
 * The credential callback the core calls each time an account's connection signs in (on a core
 * thread, never the UI thread). It reads [store], hands the core a [FfiCredential] and wipes its
 * own copy of the bytes straight after. It never logs and never throws a secret: any failure of
 * the store reads as "no credential", and the core then calls [signInNeeded].
 *
 * The core asks only about accounts the app already stored a secret for (at `open` and
 * `signedIn`). A new account's credential is not looked up here: it goes into `Core.addAccount`
 * and is stored under the returned id afterwards (see [AccountSecrets]), so the answer for one
 * account can never be another account's password.
 *
 * An OAuth access token is used when one is stored and [userFor] knows the account name it
 * belongs to (a name is not a secret, so it is kept elsewhere than here); otherwise the password.
 */
class SecretStoreCredentials(
    private val store: SecretStore,
    private val signInNeeded: SignInNeeded,
    private val userFor: (accountId: Long) -> String? = { null },
) : CredentialProvider {
    override fun credential(accountId: Long): FfiCredential? {
        try {
            oauth(accountId)?.let { return it }
            val password = store.get(accountId, SecretKind.PASSWORD) ?: return null
            return try {
                FfiCredential.password(password)
            } finally {
                password.fill(0)
            }
        } catch (e: SecretBoxException) {
            return null
        }
    }

    private fun oauth(accountId: Long): FfiCredential? {
        val user = userFor(accountId) ?: return null
        val token = store.get(accountId, SecretKind.OAUTH_ACCESS) ?: return null
        return try {
            FfiCredential.oauth2(user, token)
        } finally {
            token.fill(0)
        }
    }

    override fun signInNeeded(accountId: Long) {
        signInNeeded.mark(accountId)
    }

    override fun toString(): String = "[redacted]"
}
