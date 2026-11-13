// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import java.security.MessageDigest

/**
 * What the app keeps about this device's push subscription (T34d): the Companion's id for it, which Companion ([companion], its
 * address) and a hash of what was subscribed. Not a secret, and never the endpoint itself (for UnifiedPush, whoever has the endpoint
 * can wake the phone).
 */
data class PushRecord(val subscriptionId: Long, val companion: String, val hash: String)

/** Where the [PushRecord] is kept (a small preferences file in the app; memory in tests). */
interface PushRecords {
    fun get(): PushRecord?
    fun put(record: PushRecord)
    fun clear()
}

/** The Companion's push calls, as the connection offers them (null: not connected, or the call failed). */
interface PushCompanion {
    suspend fun addPush(endpoint: String, p256dh: String, auth: String): Long?
    suspend fun removePush(id: Long)
}

/** What S58 says about push. */
enum class PushStatus {
    /** No Companion is connected: nothing to be woken by. */
    NOT_CONNECTED,

    /** This build has no push set up (the Play build without its Firebase project and gateway): periodic checks only. */
    NOT_IN_THIS_BUILD,

    /** UnifiedPush builds: no distributor app is installed. */
    NO_DISTRIBUTOR,

    /** Asked for, not confirmed yet. */
    WAITING,

    /** The Companion can wake this phone. */
    ON,

    /** The last attempt failed; periodic checks go on. */
    FAILED,
}

/**
 * Turns a push endpoint into a subscription at the person's Companion, and a ping into a mail check (T34d). Shared by every store build:
 * only how the endpoint is obtained differs (UnifiedPush, or FCM through the gateway).
 *
 * A ping carries nothing (the Companion's pings are content-free, T28): it only makes the app check now. [checkNow] queues at most one
 * check, so a flood of pings costs one check at a time.
 */
class CompanionPush(
    private val records: PushRecords,
    private val companion: PushCompanion,
    /** The connected Companion's address: a record made at another Companion is never acted on here. */
    private val companionUrl: String,
    private val checkNow: () -> Unit,
) {
    /** The record, when it belongs to this Companion. */
    private fun ours(): PushRecord? = records.get()?.takeIf { it.companion == companionUrl }

    /**
     * An endpoint with this device's Web Push keys (base64url), new or the same as before. It is subscribed at the Companion every time
     * (the Companion replaces a subscription with the same endpoint), so a subscription the Companion dropped meanwhile (signed out,
     * delivery failures) comes back at the next start instead of S58 saying "On" with nothing behind it. A different endpoint replaces
     * the old subscription and checks for mail missed meanwhile. True when it is subscribed.
     */
    suspend fun onEndpoint(endpoint: String, p256dh: String, auth: String): Boolean {
        val hash = hashOf(endpoint, p256dh, auth)
        val old = ours()
        val id = companion.addPush(endpoint, p256dh, auth) ?: return false
        records.put(PushRecord(id, companionUrl, hash))
        if (old != null && old.subscriptionId != id && old.hash != hash) companion.removePush(old.subscriptionId)
        if (old?.hash != hash) checkNow()
        return true
    }

    /** The endpoint is no more (unregistered by the distributor, or push turned off): the subscription goes too. */
    suspend fun onEndpointGone() {
        val old = ours()
        records.clear()
        if (old != null) companion.removePush(old.subscriptionId)
    }

    /** Forgets the subscription without asking the Companion (it was signed out, which ended its subscriptions). */
    fun forget() = records.clear()

    /** Whether a subscription at this Companion is kept. */
    fun isSubscribed(): Boolean = ours() != null

    companion object {
        /** A hash of what was subscribed, so a repeat of the same endpoint is not sent again. */
        fun hashOf(endpoint: String, p256dh: String, auth: String): String {
            val d = MessageDigest.getInstance("SHA-256")
            for (part in listOf(endpoint, p256dh, auth)) {
                d.update(part.toByteArray(Charsets.UTF_8))
                d.update(0)
            }
            return d.digest().joinToString("") { "%02x".format(it) }
        }
    }
}
