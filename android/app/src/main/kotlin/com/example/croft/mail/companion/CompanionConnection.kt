// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.companion

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Where the connection's secret is kept (the keystore-backed store in the app; memory in tests). */
interface CompanionSecrets {
    fun get(): ByteArray?
    fun put(secret: ByteArray)
    fun remove()
}

/** What S06 and S58 show. */
sealed interface CompanionState {
    /** Reading what is kept. */
    data object Loading : CompanionState

    /** No Companion: S06. [problem] says why the last step did not work. */
    data class NotConnected(val problem: CompanionProblem? = null) : CompanionState

    /** The browser is open on the Companion's sign-in page. */
    data class Waiting(val url: String) : CompanionState

    /**
     * Connected: S58. [devices] is null until loaded (or when the Companion could not be reached; [problem] then says why). [userName]
     * may be empty when it was never learnt.
     */
    data class Connected(
        val url: String,
        val userName: String,
        val devices: List<CompanionDeviceInfo>? = null,
        val problem: CompanionProblem? = null,
    ) : CompanionState
}

/**
 * The app's connection to a Croft Companion (T34b): connecting through the browser (S06), what S58 shows, signing devices out and
 * disconnecting. The secret lives in [secrets] only; a Companion that ended this device's session (SIGNED_OUT) makes the app forget it.
 *
 * A redirect that arrives with no sign-in waiting for it is ignored (another app could open the Croft scheme at any time).
 */
class CompanionConnection(
    private val backend: CompanionBackend,
    private val secrets: CompanionSecrets,
    /** The app's redirect, `<applicationId>:/companion`. */
    private val redirect: String,
    /** How this device is named in the person's list. */
    private val deviceName: String,
) {
    private val _state = MutableStateFlow<CompanionState>(CompanionState.Loading)
    val state: StateFlow<CompanionState> = _state.asStateFlow()

    private val lock = Mutex()
    private var link: CompanionLinkApi? = null
    private var pendingFlow: String? = null

    private suspend fun <T> guarded(block: suspend () -> T): T? = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (e: CompanionException) {
        failed(e.problem)
        null
    } catch (e: Exception) {
        failed(CompanionProblem.STORAGE)
        null
    }

    private fun failed(problem: CompanionProblem) {
        if (problem == CompanionProblem.SIGNED_OUT) {
            forget()
            _state.value = CompanionState.NotConnected(problem)
            return
        }
        _state.value = when (val s = _state.value) {
            is CompanionState.Connected -> s.copy(problem = problem)
            else -> CompanionState.NotConnected(problem)
        }
    }

    private fun forget() {
        link = null
        try {
            secrets.remove()
        } catch (e: Exception) {
            // The next start finds it unusable and asks again.
        }
    }

    private fun linkFrom(secret: ByteArray): CompanionLinkApi =
        backend.link(secret) { next -> try { secrets.put(next) } catch (e: Exception) { /* kept in memory until the next start */ } }

    /** Reads the kept connection, and who is signed in. */
    suspend fun load() = lock.withLock {
        if (_state.value is CompanionState.Waiting) return@withLock
        val secret = try {
            secrets.get()
        } catch (e: Exception) {
            _state.value = CompanionState.NotConnected(CompanionProblem.STORAGE)
            return@withLock
        }
        if (secret == null) {
            _state.value = CompanionState.NotConnected()
            return@withLock
        }
        val l = guarded { linkFrom(secret) }.also { secret.fill(0) } ?: return@withLock
        link = l
        _state.value = CompanionState.Connected(l.url, "")
        val name = guarded { l.userName() } ?: return@withLock
        _state.value = CompanionState.Connected(l.url, name)
    }

    /** S06's Connect: checks the address and begins a sign-in; the page to open in a Custom Tab, or null (the state says why). */
    suspend fun start(typed: String): String? = lock.withLock {
        _state.value = CompanionState.NotConnected()
        val found = guarded { backend.probe(typed) } ?: return@withLock null
        val started = guarded { backend.begin(found.url, redirect) } ?: return@withLock null
        pendingFlow = started.flowId
        _state.value = CompanionState.Waiting(found.url)
        started.browserUrl
    }

    /** The person came back without finishing (closed the tab). */
    fun cancel() {
        if (_state.value is CompanionState.Waiting) {
            pendingFlow = null
            _state.value = CompanionState.NotConnected()
        }
    }

    /** The browser came back to the app's redirect with [uri]. Ignored when no sign-in is waiting. */
    suspend fun onRedirect(uri: String) = lock.withLock {
        val flow = pendingFlow ?: return@withLock
        if (!uri.startsWith("$redirect?")) return@withLock
        pendingFlow = null
        val joined = guarded { backend.finish(flow, uri, deviceName) } ?: return@withLock
        try {
            secrets.put(joined.secret)
        } catch (e: Exception) {
            joined.secret.fill(0)
            _state.value = CompanionState.NotConnected(CompanionProblem.STORAGE)
            return@withLock
        }
        val l = guarded { linkFrom(joined.secret) }.also { joined.secret.fill(0) } ?: return@withLock
        link = l
        _state.value = CompanionState.Connected(joined.url, joined.userName)
    }

    /** S58: the signed-in devices. */
    suspend fun loadDevices() = lock.withLock {
        val l = link ?: return@withLock
        val list = guarded { l.devices() } ?: return@withLock
        (_state.value as? CompanionState.Connected)?.let { _state.value = it.copy(devices = list, problem = null) }
    }

    /** S58: signs another device out, then lists them again. */
    suspend fun signOutDevice(id: Long) {
        lock.withLock {
            val l = link ?: return@withLock
            guarded { l.signOutDevice(id) }
        }
        loadDevices()
    }

    /** S58's Disconnect: the Companion ends this device's session (when it can be reached); the secret is forgotten either way. */
    suspend fun disconnect() = lock.withLock {
        val l = link
        if (l != null) {
            try {
                l.signOut()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Forgotten here anyway; the Companion's list still shows the device until it is signed out there.
            }
        }
        forget()
        _state.value = CompanionState.NotConnected()
    }
}
