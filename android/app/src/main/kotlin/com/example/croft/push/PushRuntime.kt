// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import android.content.Context
import com.example.croft.mail.companion.CompanionRuntime
import com.example.croft.notify.CheckSchedule
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** How a store build gets its endpoint (each flavour's [StorePush]). */
interface StorePushProvider {
    val kind: PushKind

    /** Asks for an endpoint for the connected Companion (its push key is [vapid]); the status to show until it arrives. */
    suspend fun start(context: Context, vapid: String): PushStatus

    /** Stops push on this device (the distributor or FCM forgets this app's registration). */
    fun stop(context: Context)
}

/** The [PushRecord] in a small private preferences file (not backed up: `croft-push` is excluded with the rest of the app's data). */
private class PrefsPushRecords(context: Context) : PushRecords {
    private val prefs = context.applicationContext.getSharedPreferences("croft-push", Context.MODE_PRIVATE)
    override fun get(): PushRecord? {
        val id = prefs.getLong("subscription", -1)
        val hash = prefs.getString("hash", null)
        return if (id < 0 || hash == null) null else PushRecord(id, hash)
    }
    override fun put(record: PushRecord) {
        prefs.edit().putLong("subscription", record.subscriptionId).putString("hash", record.hash).commit()
    }
    override fun clear() {
        prefs.edit().clear().commit()
    }
}

/** Push on this phone (T34d): what S58 shows, and the glue between the store's push, the Companion and the mail check. */
object PushRuntime {
    private val _status = MutableStateFlow(PushStatus.NOT_CONNECTED)
    val status: StateFlow<PushStatus> = _status.asStateFlow()

    fun setStatus(s: PushStatus) {
        _status.value = s
    }

    /** The registrar over the connected Companion (opening the core in a cold process), or null when none is connected. */
    suspend fun registrar(context: Context): CompanionPush? {
        val c = CompanionRuntime.openedConnection(context) ?: return null
        if (!c.isConnected()) return null
        val app = context.applicationContext
        return CompanionPush(
            PrefsPushRecords(app),
            object : PushCompanion {
                override suspend fun addPush(endpoint: String, p256dh: String, auth: String) = c.addPush(endpoint, p256dh, auth)
                override suspend fun removePush(id: Long) = c.removePush(id)
            },
        ) { CheckSchedule.now(app) }
    }

    /** Turns push on for the connected Companion (S58, and after connecting). */
    suspend fun start(context: Context) {
        val c = CompanionRuntime.openedConnection(context)
        if (c == null || !c.isConnected()) {
            _status.value = PushStatus.NOT_CONNECTED
            return
        }
        val vapid = c.pushKey()
        if (vapid == null) {
            _status.value = PushStatus.FAILED
            return
        }
        _status.value = StorePush.start(context, vapid)
    }

    /**
     * Turns push off on this device. The subscription at the Companion ends with the session when the device is disconnected (sign-out),
     * so only the local registration and record go.
     */
    fun stop(context: Context) {
        StorePush.stop(context)
        PrefsPushRecords(context).clear()
        _status.value = PushStatus.NOT_CONNECTED
    }
}
