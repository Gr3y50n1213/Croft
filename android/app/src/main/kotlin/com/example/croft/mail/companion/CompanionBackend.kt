// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.companion

import uniffi.croft_ffi.CompanionFailure
import uniffi.croft_ffi.CompanionLink
import uniffi.croft_ffi.CompanionSecretSaver
import uniffi.croft_ffi.Core

/** Why a Companion step did not work, in the app's terms (the screen owns the words). */
enum class CompanionProblem { BAD_ADDRESS, NOT_A_COMPANION, UNREACHABLE, SIGN_IN_FAILED, SIGNED_OUT, RATE_LIMITED, SERVER, STORAGE, NO_BROWSER }

/** A Companion step failed. Carries only the category, never an address or a token. */
class CompanionException(val problem: CompanionProblem) : Exception(problem.name)

/** What a Companion says about itself (S06). */
data class CompanionFound(val url: String, val version: String, val ssoName: String?)

/** A sign-in begun: the page to open in a Custom Tab. */
data class CompanionStarted(val flowId: String, val browserUrl: String)

/** A connected Companion. [secret] is kept in the keystore; never shown or logged. */
class CompanionJoined(val url: String, val userName: String, val secret: ByteArray) {
    override fun toString(): String = "CompanionJoined(url=$url)"
}

/** One of the person's signed-in devices (S58). */
data class CompanionDeviceInfo(val id: Long, val name: String, val lastSeenMs: Long, val current: Boolean)

/** The app's connection to its Companion. */
interface CompanionLinkApi {
    val url: String
    suspend fun userName(): String
    suspend fun devices(): List<CompanionDeviceInfo>
    suspend fun signOutDevice(id: Long)
    suspend fun signOut()
    /** The Companion's push (VAPID) public key, base64url (T34d). */
    suspend fun pushKey(): String
    /** Subscribes this device for pings at [endpoint] with its Web Push keys (base64url); the Companion's id for it. */
    suspend fun addPush(endpoint: String, p256dh: String, auth: String): Long
    suspend fun removePush(id: Long)
}

/** The core's Companion calls (T34b), as an interface so the state can be tested without a core. */
interface CompanionBackend {
    suspend fun probe(url: String): CompanionFound
    fun begin(url: String, redirect: String): CompanionStarted
    suspend fun finish(flowId: String, callback: String, device: String): CompanionJoined
    /** The link from a kept secret; [onNewSecret] keeps the secret the Companion replaces it with. */
    fun link(secret: ByteArray, onNewSecret: (ByteArray) -> Unit): CompanionLinkApi
}

private fun CompanionFailure.problem(): CompanionProblem = when (this) {
    is CompanionFailure.BadAddress -> CompanionProblem.BAD_ADDRESS
    is CompanionFailure.NotACompanion -> CompanionProblem.NOT_A_COMPANION
    is CompanionFailure.Unreachable -> CompanionProblem.UNREACHABLE
    is CompanionFailure.SignInFailed -> CompanionProblem.SIGN_IN_FAILED
    is CompanionFailure.SignedOut -> CompanionProblem.SIGNED_OUT
    is CompanionFailure.RateLimited -> CompanionProblem.RATE_LIMITED
    else -> CompanionProblem.SERVER
}

private inline fun <T> mapped(block: () -> T): T = try {
    block()
} catch (e: CompanionFailure) {
    throw CompanionException(e.problem())
}

/** [CompanionBackend] over the core (FFI v14). */
class CoreCompanionBackend(private val core: Core) : CompanionBackend {
    override suspend fun probe(url: String): CompanionFound = mapped {
        val info = core.companionProbe(url)
        CompanionFound(info.url, info.version, info.ssoName)
    }

    override fun begin(url: String, redirect: String): CompanionStarted = mapped {
        val s = core.companionBegin(url, redirect)
        CompanionStarted(s.flowId, s.browserUrl)
    }

    override suspend fun finish(flowId: String, callback: String, device: String): CompanionJoined = mapped {
        val c = core.companionFinish(flowId, callback, device)
        CompanionJoined(c.url, c.userName, c.secret)
    }

    override fun link(secret: ByteArray, onNewSecret: (ByteArray) -> Unit): CompanionLinkApi = mapped {
        val saver = object : CompanionSecretSaver {
            override fun save(secret: ByteArray) {
                try {
                    onNewSecret(secret)
                } finally {
                    secret.fill(0)
                }
            }
        }
        CoreLink(core.companionLink(secret, saver))
    }

    private class CoreLink(private val link: CompanionLink) : CompanionLinkApi {
        override val url: String get() = link.url()
        override suspend fun userName(): String = mapped { link.userName() }
        override suspend fun devices(): List<CompanionDeviceInfo> = mapped {
            link.devices().map { CompanionDeviceInfo(it.id, it.name, it.lastSeenMs, it.current) }
        }
        override suspend fun signOutDevice(id: Long) = mapped { link.signOutDevice(id) }
        override suspend fun signOut() = mapped { link.signOut() }
        override suspend fun pushKey(): String = mapped { link.pushKey() }
        override suspend fun addPush(endpoint: String, p256dh: String, auth: String): Long = mapped { link.addPush(endpoint, p256dh, auth) }
        override suspend fun removePush(id: Long) = mapped { link.removePush(id) }
    }
}
