// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.companion

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val REDIRECT = "com.example.croft:/companion"

/** A made-up Companion: one person, "ann", and secrets that are plain made-up bytes. */
private class FakeBackend : CompanionBackend {
    var probeProblem: CompanionProblem? = null
    var finishProblem: CompanionProblem? = null
    var linkProblem: CompanionProblem? = null
    val finished = mutableListOf<Pair<String, String>>()
    var devices = mutableListOf(
        CompanionDeviceInfo(1, "Croft on this phone", 10, current = true),
        CompanionDeviceInfo(2, "Chrome (web)", 20, current = false),
    )
    var signedOut = false
    var rotateTo: ByteArray? = null
    var links = 0

    override suspend fun probe(url: String): CompanionFound {
        probeProblem?.let { throw CompanionException(it) }
        return CompanionFound("https://companion.example.org", "0.1.0", null)
    }

    override fun begin(url: String, redirect: String) = CompanionStarted("flow-1", "$url/app/sign-in?app_challenge=x")

    override suspend fun finish(flowId: String, callback: String, device: String): CompanionJoined {
        finished += flowId to callback
        finishProblem?.let { throw CompanionException(it) }
        return CompanionJoined("https://companion.example.org", "ann", "made-up-secret".toByteArray())
    }

    override fun link(secret: ByteArray, onNewSecret: (ByteArray) -> Unit): CompanionLinkApi {
        linkProblem?.let { throw CompanionException(it) }
        links++
        val backend = this
        return object : CompanionLinkApi {
            override val url = "https://companion.example.org"
            override suspend fun userName(): String {
                rotateTo?.let { onNewSecret(it) }
                if (backend.signedOut) throw CompanionException(CompanionProblem.SIGNED_OUT)
                return "ann"
            }
            override suspend fun devices(): List<CompanionDeviceInfo> = backend.devices.toList()
            override suspend fun signOutDevice(id: Long) { backend.devices.removeAll { it.id == id } }
            override suspend fun signOut() { backend.signedOut = true }
        }
    }
}

private class MemorySecrets : CompanionSecrets {
    var value: ByteArray? = null
    var failPuts = false
    override fun get() = value?.copyOf()
    override fun put(secret: ByteArray) {
        if (failPuts) throw java.io.IOException("made-up storage failure")
        value = secret.copyOf()
    }
    override fun remove() { value = null }
}

class CompanionConnectionTest {
    private val backend = FakeBackend()
    private val secrets = MemorySecrets()
    private val connection = CompanionConnection(backend, secrets, REDIRECT, "Croft on this phone")

    @Test
    fun withNothingKeptItIsNotConnected() = runBlocking<Unit> {
        connection.load()
        assertEquals(CompanionState.NotConnected(), connection.state.value)
    }

    @Test
    fun connectingOpensThePageAndTheRedirectFinishesIt() = runBlocking<Unit> {
        connection.load()
        val page = connection.start("companion.example.org")
        assertEquals("https://companion.example.org/app/sign-in?app_challenge=x", page)
        assertEquals(CompanionState.Waiting("https://companion.example.org"), connection.state.value)
        connection.onRedirect("$REDIRECT?code=made-up-code")
        assertEquals(listOf("flow-1" to "$REDIRECT?code=made-up-code"), backend.finished)
        assertEquals(CompanionState.Connected("https://companion.example.org", "ann"), connection.state.value)
        assertArrayEquals("made-up-secret".toByteArray(), secrets.value)
        // A second redirect (another app opening the scheme) does nothing.
        connection.onRedirect("$REDIRECT?code=another")
        assertEquals(1, backend.finished.size)
    }

    @Test
    fun aRedirectWithNoSignInWaitingIsIgnored() = runBlocking<Unit> {
        connection.load()
        connection.onRedirect("$REDIRECT?code=from-another-app")
        assertTrue(backend.finished.isEmpty())
        assertNull(secrets.value)
        assertEquals(CompanionState.NotConnected(), connection.state.value)
    }

    @Test
    fun aRedirectForAnotherAddressOrAfterCancelIsIgnored() = runBlocking<Unit> {
        connection.start("companion.example.org")
        connection.onRedirect("com.evil.app:/companion?code=x")
        assertTrue(backend.finished.isEmpty())
        connection.cancel()
        connection.onRedirect("$REDIRECT?code=late")
        assertTrue(backend.finished.isEmpty())
        assertEquals(CompanionState.NotConnected(), connection.state.value)
    }

    @Test
    fun aProblemSaysWhyAndKeepsNothing() = runBlocking<Unit> {
        backend.probeProblem = CompanionProblem.NOT_A_COMPANION
        assertNull(connection.start("example.org"))
        assertEquals(CompanionState.NotConnected(CompanionProblem.NOT_A_COMPANION), connection.state.value)
        backend.probeProblem = null
        backend.finishProblem = CompanionProblem.SIGN_IN_FAILED
        connection.start("companion.example.org")
        connection.onRedirect("$REDIRECT?code=spent")
        assertEquals(CompanionState.NotConnected(CompanionProblem.SIGN_IN_FAILED), connection.state.value)
        assertNull(secrets.value)
    }

    @Test
    fun aKeptConnectionLoadsListsDevicesAndSignsOneOut() = runBlocking<Unit> {
        secrets.value = "made-up-secret".toByteArray()
        connection.load()
        assertEquals(CompanionState.Connected("https://companion.example.org", "ann"), connection.state.value)
        connection.loadDevices()
        assertEquals(2, (connection.state.value as CompanionState.Connected).devices!!.size)
        connection.signOutDevice(2)
        assertEquals(listOf(1L), (connection.state.value as CompanionState.Connected).devices!!.map { it.id })
    }

    @Test
    fun aReplacedSecretIsKept() = runBlocking<Unit> {
        secrets.value = "made-up-secret".toByteArray()
        backend.rotateTo = "made-up-secret-2".toByteArray()
        connection.load()
        assertArrayEquals("made-up-secret-2".toByteArray(), secrets.value)
    }

    @Test
    fun signedOutByTheCompanionForgetsTheSecret() = runBlocking<Unit> {
        secrets.value = "made-up-secret".toByteArray()
        backend.signedOut = true
        connection.load()
        assertEquals(CompanionState.NotConnected(CompanionProblem.SIGNED_OUT), connection.state.value)
        assertNull(secrets.value)
    }

    @Test
    fun disconnectingSignsOutAndForgets() = runBlocking<Unit> {
        secrets.value = "made-up-secret".toByteArray()
        connection.load()
        connection.disconnect()
        assertTrue(backend.signedOut)
        assertNull(secrets.value)
        assertEquals(CompanionState.NotConnected(), connection.state.value)
    }

    @Test
    fun aCompanionThatCannotBeReachedKeepsTheConnection() = runBlocking<Unit> {
        secrets.value = "made-up-secret".toByteArray()
        val offline = object : CompanionBackend by backend {
            override fun link(secret: ByteArray, onNewSecret: (ByteArray) -> Unit): CompanionLinkApi {
                val real = backend.link(secret, onNewSecret)
                return object : CompanionLinkApi by real {
                    override suspend fun userName(): String = throw CompanionException(CompanionProblem.UNREACHABLE)
                }
            }
        }
        val c = CompanionConnection(offline, secrets, REDIRECT, "Croft on this phone")
        c.load()
        assertEquals(
            CompanionState.Connected("https://companion.example.org", "", problem = CompanionProblem.UNREACHABLE),
            c.state.value,
        )
        // Still kept: only being signed out by the Companion forgets it.
        assertArrayEquals("made-up-secret".toByteArray(), secrets.value)
    }

    @Test
    fun aSecondVisitUsesTheOpenLinkNotTheStore() = runBlocking<Unit> {
        secrets.value = "made-up-secret".toByteArray()
        connection.load()
        // Even if the store still held an older secret, the open link (with the newest tokens) is used.
        secrets.value = "made-up-stale".toByteArray()
        connection.load()
        assertEquals(1, backend.links)
        assertEquals(CompanionState.Connected("https://companion.example.org", "ann"), connection.state.value)
    }

    @Test
    fun aSecretThatCouldNotBeSavedIsSavedAtTheNextStep() = runBlocking<Unit> {
        secrets.value = "made-up-secret".toByteArray()
        backend.rotateTo = "made-up-secret-2".toByteArray()
        secrets.failPuts = true
        connection.load()
        assertArrayEquals("made-up-secret".toByteArray(), secrets.value)
        secrets.failPuts = false
        backend.rotateTo = null
        connection.load()
        assertArrayEquals("made-up-secret-2".toByteArray(), secrets.value)
        assertEquals(1, backend.links)
    }

    @Test
    fun aRefusedCodeLeavesTheSignInWaitingForTheRealOne() = runBlocking<Unit> {
        connection.start("companion.example.org")
        backend.finishProblem = CompanionProblem.SIGN_IN_FAILED
        connection.onRedirect("$REDIRECT?code=from-another-app")
        assertEquals(CompanionState.NotConnected(CompanionProblem.SIGN_IN_FAILED), connection.state.value)
        backend.finishProblem = null
        connection.onRedirect("$REDIRECT?code=the-real-one")
        assertEquals(CompanionState.Connected("https://companion.example.org", "ann"), connection.state.value)
        assertEquals(listOf("flow-1", "flow-1"), backend.finished.map { it.first })
    }

    @Test
    fun aNewSessionThatCannotBeKeptIsSignedOut() = runBlocking<Unit> {
        secrets.failPuts = true
        connection.start("companion.example.org")
        connection.onRedirect("$REDIRECT?code=made-up-code")
        assertEquals(CompanionState.NotConnected(CompanionProblem.STORAGE), connection.state.value)
        assertTrue(backend.signedOut)
        assertNull(secrets.value)
    }
}
