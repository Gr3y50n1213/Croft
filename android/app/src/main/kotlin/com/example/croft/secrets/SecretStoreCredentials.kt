// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.util.concurrent.atomic.AtomicReference
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
 * An OAuth access token is used when one is stored and [userFor] knows the account name it
 * belongs to (a name is not a secret, so it is kept elsewhere than here); otherwise the password.
 */
class SecretStoreCredentials(
    private val store: SecretStore,
    private val signInNeeded: SignInNeeded,
    private val userFor: (accountId: Long) -> String? = { null },
) : CredentialProvider {
    private val staged = AtomicReference<ByteArray?>(null)

    override fun credential(accountId: Long): FfiCredential? {
        try {
            oauth(accountId)?.let { return it }
            val password = store.get(accountId, SecretKind.PASSWORD)
                ?: return adoptStaged(accountId)
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

    /**
     * For a new account: the core asks for the credential of the new row id before `addAccount`
     * returns, and the app has no stored secret under that id yet. Inside [block] (put the
     * `addAccount` call there) the first such question is answered with [password], which is also
     * stored under that id. [password] is copied; the copy is wiped when [block] ends, used or
     * not. Wipe [password] yourself afterwards.
     */
    fun <R> withStagedPassword(password: ByteArray, block: () -> R): R {
        val copy = password.copyOf()
        staged.getAndSet(copy)?.fill(0)
        try {
            return block()
        } finally {
            staged.getAndSet(null)?.fill(0)
        }
    }

    private fun adoptStaged(accountId: Long): FfiCredential? {
        val secret = staged.getAndSet(null) ?: return null
        try {
            store.put(accountId, SecretKind.PASSWORD, secret)
            return FfiCredential.password(secret)
        } finally {
            secret.fill(0)
        }
    }

    override fun toString(): String = "[redacted]"
}
