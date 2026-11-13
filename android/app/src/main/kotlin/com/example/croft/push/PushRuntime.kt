// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import android.content.Context
import com.example.croft.mail.companion.CompanionRuntime
import com.example.croft.notify.CheckSchedule
import java.util.Base64
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** What a store build's push answers when asked to start. */
sealed interface StoreStart {
    /** The endpoint comes later (UnifiedPush: to [CroftPushService]); show this meanwhile. */
    data class Status(val status: PushStatus) : StoreStart

    /** The endpoint, now (FCM through the gateway). */
    data class Endpoint(val url: String, val p256dh: String, val auth: String) : StoreStart
}

/** How a store build gets its endpoint (each flavour's [StorePush]). Called under [PushRuntime]'s lock, on the IO dispatcher. */
interface StorePushProvider {
    val kind: PushKind

    /** Asks for an endpoint for the connected Companion (its push key is [vapid], already checked). */
    suspend fun start(context: Context, vapid: String): StoreStart

    /** Stops push on this device (the distributor or FCM forgets this app's registration). */
    fun stop(context: Context)
}

/** The [PushRecord] in a small private preferences file (preferences are excluded from backups with the rest of the app's data). */
private class PrefsPushRecords(context: Context) : PushRecords {
    private val prefs = context.applicationContext.getSharedPreferences("croft-push", Context.MODE_PRIVATE)
    override fun get(): PushRecord? {
        val id = prefs.getLong("subscription", -1)
        val companion = prefs.getString("companion", null)
        val hash = prefs.getString("hash", null)
        return if (id < 0 || companion == null || hash == null) null else PushRecord(id, companion, hash)
    }
    override fun put(record: PushRecord) {
        prefs.edit().putLong("subscription", record.subscriptionId).putString("companion", record.companion)
            .putString("hash", record.hash).commit()
    }
    override fun clear() {
        prefs.edit().clear().commit()
    }
}

/**
 * Push on this phone (T34d): what S58 shows, and the glue between the store's push, the Companion and the mail check. Starting,
 * a new endpoint, a gone endpoint and stopping run one at a time (one lock, on the IO dispatcher), so two of them never subscribe
 * twice or write a record after a stop.
 */
object PushRuntime {
    private val _status = MutableStateFlow(PushStatus.NOT_CONNECTED)
    val status: StateFlow<PushStatus> = _status.asStateFlow()

    private val lock = Mutex()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    fun setStatus(s: PushStatus) {
        _status.value = s
    }

    /** The registrar over the connected Companion (opening the core in a cold process), or null when none is connected. */
    private suspend fun registrar(context: Context): CompanionPush? {
        val c = CompanionRuntime.openedConnection(context) ?: return null
        if (!c.isConnected()) return null
        val url = c.companionUrl() ?: return null
        val app = context.applicationContext
        return CompanionPush(
            PrefsPushRecords(app),
            object : PushCompanion {
                override suspend fun addPush(endpoint: String, p256dh: String, auth: String) = c.addPush(endpoint, p256dh, auth)
                override suspend fun removePush(id: Long) = c.removePush(id)
            },
            url,
        ) { CheckSchedule.now(app) }
    }

    /** Turns push on for the connected Companion (S58, after connecting, and at each app start). */
    suspend fun start(context: Context) = withContext(Dispatchers.IO) {
        lock.withLock {
            val c = CompanionRuntime.openedConnection(context)
            if (c == null || !c.isConnected()) {
                _status.value = PushStatus.NOT_CONNECTED
                return@withLock
            }
            val vapid = c.pushKey()?.takeIf(::isVapidKey)
            if (vapid == null) {
                _status.value = PushStatus.FAILED
                return@withLock
            }
            val answer = try {
                StorePush.start(context, vapid)
            } catch (e: Exception) {
                StoreStart.Status(PushStatus.FAILED)
            }
            _status.value = when (answer) {
                is StoreStart.Status -> answer.status
                is StoreStart.Endpoint -> subscribe(context, answer.url, answer.p256dh, answer.auth)
            }
        }
    }

    /** A new endpoint from the store's push (UnifiedPush's service, a new FCM token). */
    suspend fun onEndpoint(context: Context, url: String, p256dh: String, auth: String) = withContext(Dispatchers.IO) {
        lock.withLock { _status.value = subscribe(context, url, p256dh, auth) }
    }

    private suspend fun subscribe(context: Context, url: String, p256dh: String, auth: String): PushStatus {
        val push = registrar(context) ?: return PushStatus.NOT_CONNECTED
        return if (push.onEndpoint(url, p256dh, auth)) PushStatus.ON else PushStatus.FAILED
    }

    /** The store's push dropped the endpoint (the distributor unregistered this app). */
    suspend fun onEndpointGone(context: Context) = withContext(Dispatchers.IO) {
        lock.withLock {
            registrar(context)?.onEndpointGone()
            _status.value = PushStatus.FAILED
        }
    }

    /**
     * A ping. [authentic] says it was proven to come from the Companion (it decrypted with this device's keys, or it came through FCM,
     * which only the gateway can send to and which forwards only messages the Companion signed); anything else is ignored. The check is
     * queued at once, before anything slow, so a process ended soon after still has it.
     */
    fun onPing(context: Context, authentic: Boolean) {
        if (authentic) CheckSchedule.now(context.applicationContext)
    }

    /**
     * Turns push off on this device. The subscription at the Companion ends with the session when the device is unregistered (sign-out),
     * so only the local registration and records go.
     */
    suspend fun stop(context: Context) = withContext(Dispatchers.IO) {
        lock.withLock {
            try {
                StorePush.stop(context)
            } catch (e: Exception) {
                // The registration stays with the distributor or FCM; with no subscription at the Companion nothing is sent to it.
            }
            PrefsPushRecords(context).clear()
            _status.value = PushStatus.NOT_CONNECTED
        }
    }

    /** [stop] from code that cannot wait (the connection forgetting a session the Companion ended). */
    fun stopLater(context: Context) {
        val app = context.applicationContext
        scope.launch { stop(app) }
    }

    /** A VAPID public key as the Companion gives it: base64url of an uncompressed P-256 point (65 bytes, starting with 4). */
    fun isVapidKey(key: String): Boolean = try {
        val b = Base64.getUrlDecoder().decode(key)
        b.size == 65 && b[0] == 4.toByte()
    } catch (e: IllegalArgumentException) {
        false
    }
}
