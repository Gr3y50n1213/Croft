// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private class MemoryRecords : PushRecords {
    var record: PushRecord? = null
    override fun get() = record
    override fun put(record: PushRecord) { this.record = record }
    override fun clear() { record = null }
}

/** Like the Companion (push_store.rs `add_push`): a subscription with the same endpoint is replaced, under a new id. */
private class FakeCompanion : PushCompanion {
    var connected = true
    val subscriptions = mutableMapOf<Long, String>()
    val removed = mutableListOf<Long>()
    private var next = 1L
    override suspend fun addPush(endpoint: String, p256dh: String, auth: String): Long? {
        if (!connected) return null
        subscriptions.values.removeAll { it == endpoint }
        subscriptions[next] = endpoint
        return next++
    }
    override suspend fun removePush(id: Long) {
        removed += id
        subscriptions.remove(id)
    }
}

class CompanionPushTest {
    private val records = MemoryRecords()
    private val companion = FakeCompanion()
    private var checks = 0
    private fun push(url: String = "https://companion.example.org") = CompanionPush(records, companion, url) { checks++ }

    @Test
    fun theSameEndpointIsSubscribedAgainEachTimeAndChecksOnlyWhenNew() = runBlocking<Unit> {
        assertTrue(push().onEndpoint("https://push.example.org/a", "made-up-key", "made-up-auth"))
        assertTrue(push().onEndpoint("https://push.example.org/a", "made-up-key", "made-up-auth"))
        assertEquals("one subscription at the Companion", listOf("https://push.example.org/a"), companion.subscriptions.values.toList())
        assertEquals("a new endpoint checks for mail missed meanwhile, a repeat does not", 1, checks)
        assertEquals(2L, records.record!!.subscriptionId)
    }

    @Test
    fun aSubscriptionTheCompanionDroppedComesBack() = runBlocking<Unit> {
        push().onEndpoint("https://push.example.org/a", "k", "a")
        companion.subscriptions.clear() // signed out, or dropped after delivery failures
        assertTrue(push().onEndpoint("https://push.example.org/a", "k", "a"))
        assertEquals(listOf("https://push.example.org/a"), companion.subscriptions.values.toList())
    }

    @Test
    fun aNewEndpointReplacesTheOld() = runBlocking<Unit> {
        push().onEndpoint("https://push.example.org/a", "k", "a")
        push().onEndpoint("https://push.example.org/b", "k", "a")
        assertEquals(listOf("https://push.example.org/b"), companion.subscriptions.values.toList())
        assertEquals(2, checks)
    }

    @Test
    fun aRecordFromAnotherCompanionIsNeverActedOnThere() = runBlocking<Unit> {
        push("https://old.example.org").onEndpoint("https://push.example.org/a", "k", "a")
        val oldId = records.record!!.subscriptionId
        push("https://new.example.org").onEndpoint("https://push.example.org/b", "k", "a")
        assertFalse("the old id means nothing at the new Companion", companion.removed.contains(oldId))
        assertEquals("https://new.example.org", records.record!!.companion)
        records.record = records.record!!.copy(companion = "https://old.example.org")
        push("https://new.example.org").onEndpointGone()
        assertTrue(companion.removed.isEmpty())
        assertNull(records.record)
    }

    @Test
    fun theEndpointItselfIsNeverKept() = runBlocking<Unit> {
        push().onEndpoint("https://push.example.org/secret-capability", "made-up-key", "made-up-auth")
        assertFalse(records.record.toString().contains("secret-capability"))
    }

    @Test
    fun withoutACompanionNothingIsKept() = runBlocking<Unit> {
        companion.connected = false
        assertFalse(push().onEndpoint("https://push.example.org/a", "k", "a"))
        assertNull(records.record)
        assertEquals(0, checks)
    }

    @Test
    fun aGoneEndpointRemovesTheSubscription() = runBlocking<Unit> {
        push().onEndpoint("https://push.example.org/a", "k", "a")
        push().onEndpointGone()
        assertTrue(companion.subscriptions.isEmpty())
        assertFalse(push().isSubscribed())
        push().onEndpointGone() // nothing kept: nothing to do
    }

    @Test
    fun onlyAVapidKeyOfTheRightShapeIsUsed() {
        val point = ByteArray(65).also { it[0] = 4 }
        val good = java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(point)
        assertTrue(PushRuntime.isVapidKey(good))
        assertFalse(PushRuntime.isVapidKey(good.drop(4)))
        assertFalse(PushRuntime.isVapidKey("not base64 !"))
        point[0] = 2
        assertFalse(PushRuntime.isVapidKey(java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(point)))
    }
}
