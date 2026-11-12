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

    override fun saveTokens(accountId: Long, secret: ByteArray) { secret.fill(0) }
}

/**
 * The app's credential store, in memory. Like the Android one it answers only for ids the app
 * stored ([store], which a test calls right after `addAccount` returns, as the app does).
 */
class MemoryCredentials : CredentialProvider {
    private val stored = ConcurrentHashMap<Long, FfiCredential>()
    val asked = CopyOnWriteArrayList<Long>()
    val signIns = CopyOnWriteArrayList<Long>()

    fun store(accountId: Long, credential: FfiCredential) {
        stored[accountId] = credential
    }

    override fun credential(accountId: Long): FfiCredential? {
        asked.add(accountId)
        return stored[accountId]
    }

    override fun signInNeeded(accountId: Long) {
        signIns.add(accountId)
    }

    override fun saveTokens(accountId: Long, secret: ByteArray) { secret.fill(0) }
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

    override fun saveTokens(accountId: Long, secret: ByteArray) {
        throw IllegalStateException("save failed: $SECRET_TEXT")
    }

    companion object {
        /** Stands for text from an exception that must not reach any Rust-side message. */
        const val SECRET_TEXT = "exception-text-Zq4-6612"
    }
}
