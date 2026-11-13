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

private class FakeCompanion : PushCompanion {
    var connected = true
    val subscriptions = mutableMapOf<Long, String>()
    private var next = 1L
    override suspend fun addPush(endpoint: String, p256dh: String, auth: String): Long? {
        if (!connected) return null
        subscriptions[next] = endpoint
        return next++
    }
    override suspend fun removePush(id: Long) { subscriptions.remove(id) }
}

class CompanionPushTest {
    private val records = MemoryRecords()
    private val companion = FakeCompanion()
    private var checks = 0
    private val push = CompanionPush(records, companion) { checks++ }

    @Test
    fun anEndpointIsSubscribedOnceAndAReplacementRemovesTheOld() = runBlocking<Unit> {
        assertTrue(push.onEndpoint("https://push.example.org/a", "made-up-key", "made-up-auth"))
        assertTrue(push.onEndpoint("https://push.example.org/a", "made-up-key", "made-up-auth"))
        assertEquals(mapOf(1L to "https://push.example.org/a"), companion.subscriptions)
        assertEquals("a new endpoint checks for mail missed meanwhile", 1, checks)
        assertTrue(push.onEndpoint("https://push.example.org/b", "made-up-key", "made-up-auth"))
        assertEquals(mapOf(2L to "https://push.example.org/b"), companion.subscriptions)
        assertEquals(2L, records.record!!.subscriptionId)
    }

    @Test
    fun theEndpointItselfIsNeverKept() = runBlocking<Unit> {
        push.onEndpoint("https://push.example.org/secret-capability", "made-up-key", "made-up-auth")
        assertFalse(records.record.toString().contains("secret-capability"))
    }

    @Test
    fun withoutACompanionNothingIsKept() = runBlocking<Unit> {
        companion.connected = false
        assertFalse(push.onEndpoint("https://push.example.org/a", "k", "a"))
        assertNull(records.record)
        assertEquals(0, checks)
    }

    @Test
    fun aGoneEndpointRemovesTheSubscription() = runBlocking<Unit> {
        push.onEndpoint("https://push.example.org/a", "k", "a")
        push.onEndpointGone()
        assertTrue(companion.subscriptions.isEmpty())
        assertFalse(push.isSubscribed())
        push.onEndpointGone() // nothing kept: nothing to do
    }

    @Test
    fun onlyAnAuthenticPingChecks() {
        push.onPing(authentic = false)
        assertEquals(0, checks)
        push.onPing(authentic = true)
        assertEquals(1, checks)
    }
}
