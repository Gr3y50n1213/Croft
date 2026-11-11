// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import uniffi.croft_ffi.CredentialProvider
import uniffi.croft_ffi.FfiCredential

/** A provider with no credentials at all: every account starts in the needs-sign-in state. */
class NoCredentials : CredentialProvider {
    override fun credential(accountId: Long): FfiCredential? = null

    override fun signInNeeded(accountId: Long) {}
}

/**
 * The app's credential store, in memory. [pending] is the secret the person just verified: the
 * first time the core asks about an id nobody has stored yet, it is stored under that id (what the
 * Android keystore implementation does when `addAccount` asks).
 */
class MemoryCredentials : CredentialProvider {
    private val stored = ConcurrentHashMap<Long, FfiCredential>()
    val asked = CopyOnWriteArrayList<Long>()
    val signIns = CopyOnWriteArrayList<Long>()

    @Volatile
    var pending: FfiCredential? = null

    fun store(accountId: Long, credential: FfiCredential) {
        stored[accountId] = credential
    }

    override fun credential(accountId: Long): FfiCredential? {
        asked.add(accountId)
        stored[accountId]?.let { return it }
        val p = pending ?: return null
        pending = null
        stored[accountId] = p
        return p
    }

    override fun signInNeeded(accountId: Long) {
        signIns.add(accountId)
    }
}

/** A provider whose every callback throws, as a keystore failure in the app would. */
class ThrowingCredentials : CredentialProvider {
    val asked = AtomicInteger()

    override fun credential(accountId: Long): FfiCredential? {
        asked.incrementAndGet()
        throw IllegalStateException("keystore failed: $SECRET_TEXT")
    }

    override fun signInNeeded(accountId: Long) {
        asked.incrementAndGet()
        throw IllegalStateException("sign-in hand-off failed: $SECRET_TEXT")
    }

    companion object {
        /** Stands for text from an exception that must not reach any Rust-side message. */
        const val SECRET_TEXT = "exception-text-Zq4-6612"
    }
}
