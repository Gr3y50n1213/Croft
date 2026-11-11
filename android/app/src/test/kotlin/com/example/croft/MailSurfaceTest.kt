// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import java.io.File
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import uniffi.croft_ffi.AccountStatus
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.DiscoverException
import uniffi.croft_ffi.DiscoveryObserver
import uniffi.croft_ffi.DiscoveryProbe
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.MessageScope
import uniffi.croft_ffi.ProbeProgress
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings
import uniffi.croft_ffi.VerifyFailure
import uniffi.croft_ffi.discoverErrorMessageKey
import uniffi.croft_ffi.verifyFailureDefaultMessage
import uniffi.croft_ffi.verifyFailureMessageKey

/**
 * The mail surface of the Rust core through the generated bindings, on the JVM. These flows need no
 * mail server: the account's server is a port nobody listens on, so the core's sync fails quietly
 * and everything the screens see comes from the local database. The flows against a real (in-process)
 * IMAP server are the Rust tests in core/ffi/src/tests, which call the same exported functions.
 */
class MailSurfaceTest {
    private lateinit var dir: File
    private val open = mutableListOf<Core>()

    private val password = "fixture-pw-Kt-7731"

    @Before
    fun setUp() {
        dir = createTempDirectory("croft-mail-test").toFile()
    }

    @After
    fun tearDown() {
        open.forEach { it.close() }
        dir.deleteRecursively()
    }

    private fun db() = File(dir, "croft.db").path

    /** What the person typed, as the credential `addAccount` takes. */
    private fun typed(text: String = password) = FfiCredential.password(text.toByteArray())

    private fun openCore(provider: uniffi.croft_ffi.CredentialProvider): Core =
        Core.open(db(), null, provider).also { open += it }

    /** Settings for a server that is not there: port 1 on this computer, with TLS. */
    private fun deadServer() =
        FfiDiscovered(
            email = "alice@example.org",
            imap = ServerSettings("127.0.0.1", 1u, SecurityMode.TLS, "alice"),
            smtp = null,
            jmap = null,
            source = DiscoverySource.AUTOCONFIG,
            provider = null,
            authHint = AuthKind.PASSWORD,
        )

    private fun <T> await(what: String, check: () -> T?): T {
        val end = System.currentTimeMillis() + 10_000
        while (System.currentTimeMillis() < end) {
            check()?.let { return it }
            Thread.sleep(50)
        }
        fail("timed out waiting for: $what")
        error("unreachable")
    }

    @Test
    fun openWithNoKeyAndReopen() {
        val provider = MemoryCredentials()
        val first = openCore(provider)
        assertTrue(first.listAccounts().isEmpty())
        first.close()
        open.remove(first)
        val second = openCore(provider)
        assertTrue(second.listAccounts().isEmpty())
        assertTrue(provider.asked.isEmpty())
    }

    @Test
    fun aKeyOfTheWrongLengthIsATypedStorageError() {
        try {
            Core.open(db(), byteArrayOf(1, 2, 3), NoCredentials())
            fail("expected a CoreException")
        } catch (e: CoreException.Storage) {
            // InvalidKey with SQLCipher, EncryptionUnavailable without: either way a fixed reason.
            assertNotNull(e.reason)
        }
    }

    @Test
    fun addListAndRemoveAnAccount() {
        val provider = MemoryCredentials()
        val core = openCore(provider)
        val id = runBlocking { core.addAccount(deadServer(), "Alice", null, null, typed()).id }

        // The credential came with addAccount: the app's provider is not asked about a new account.
        assertTrue(provider.asked.isEmpty())
        val accounts = core.listAccounts()
        assertEquals(1, accounts.size)
        assertEquals(id, accounts[0].id)
        assertEquals("Alice", accounts[0].label)
        assertEquals("alice@example.org", accounts[0].email)
        assertEquals(AccountStatus.OK, accounts[0].status)

        // Nothing synced (nobody is listening), so the local lists are empty.
        assertTrue(core.listFolders(id).isEmpty())
        val page = core.listMessages(MessageScope.UnifiedInbox, null, 50u)
        assertTrue(page.rows.isEmpty())
        assertNull(page.next)

        runBlocking { core.removeAccount(id) }
        assertTrue(core.listAccounts().isEmpty())
        try {
            runBlocking { core.removeAccount(id) }
            fail("expected NoSuchAccount")
        } catch (e: CoreException.NoSuchAccount) {
            // fixed category, nothing else
        }
    }

    @Test
    fun anAccountSurvivesAReopenAndTheProviderIsAskedAgain() {
        val provider = MemoryCredentials()
        val first = openCore(provider)
        val id = runBlocking { first.addAccount(deadServer(), "Alice", null, null, typed()).id }
        // What the app does right after addAccount returns.
        provider.store(id, typed())
        first.close()
        open.remove(first)

        val second = openCore(provider)
        assertEquals(listOf(id), second.listAccounts().map { it.id })
        assertEquals(listOf(id), provider.asked.toList())
    }

    @Test
    fun noCredentialMeansNeedsSignInAndTheProviderIsTold() {
        val provider = MemoryCredentials() // nothing stored
        val first = openCore(provider)
        val id = runBlocking { first.addAccount(deadServer(), "Alice", null, null, typed()).id }
        first.close()
        open.remove(first)

        // A new run where the app has no secret for the stored account.
        val core = openCore(provider)
        assertEquals(AccountStatus.NEEDS_SIGN_IN, core.listAccounts()[0].status)
        await("signInNeeded") { provider.signIns.firstOrNull() }
        assertEquals(listOf(id), provider.signIns.toList())

        // Still nothing stored: a typed error, and the account stays paused.
        try {
            runBlocking { core.signedIn(id) }
            fail("expected NoCredential")
        } catch (e: CoreException.NoCredential) {
            // expected
        }
        assertEquals(AccountStatus.NEEDS_SIGN_IN, core.listAccounts()[0].status)

        // The person fixed it: the app stores the secret and says so.
        provider.store(id, FfiCredential.password(password.toByteArray()))
        runBlocking { core.signedIn(id) }
        assertEquals(AccountStatus.OK, core.listAccounts()[0].status)
    }

    @Test
    fun nextChangeReturnsWhatChangedAndWaitsForMore() {
        val provider = MemoryCredentials()
        val core = openCore(provider)
        val id = runBlocking { core.addAccount(deadServer(), "Alice", null, null, typed()).id }
        val batch = runBlocking { withTimeout(10_000) { core.nextChange(0u) } }
        assertTrue(batch.accounts.contains(id))
        assertFalse(batch.everything)

        // Going offline is a change of the account's status chip.
        core.setNetworkAvailable(false)
        val next = runBlocking { withTimeout(10_000) { core.nextChange(batch.seq) } }
        assertTrue(next.seq > batch.seq)
        assertTrue(next.accounts.contains(id))
        assertEquals(AccountStatus.OFFLINE, core.listAccounts()[0].status)
        core.setNetworkAvailable(true)
        assertEquals(AccountStatus.OK, core.listAccounts()[0].status)
    }

    @Test
    fun verifyAndDiscoverReportTypedFailures() {
        val core = openCore(NoCredentials())
        val cred = FfiCredential.password(password.toByteArray())

        // Nobody listening: a typed failure with a message key, not a string with data.
        try {
            runBlocking { core.verify(deadServer(), cred, null) }
            fail("expected a VerifyFailure")
        } catch (e: VerifyFailure.CannotConnect) {
            assertEquals("verify.cannot_connect", verifyFailureMessageKey(e))
            assertTrue(verifyFailureDefaultMessage(e).isNotEmpty())
        }
        try {
            runBlocking { core.verify(deadServer(), cred, "not a fingerprint") }
            fail("expected BadSettings")
        } catch (e: VerifyFailure.BadSettings) {
            assertEquals("verify.bad_settings", verifyFailureMessageKey(e))
        }
        try {
            runBlocking { core.verify(deadServer().copy(imap = null), cred, null) }
            fail("expected JmapOnly")
        } catch (e: VerifyFailure.JmapOnly) {
            // expected
        }

        try {
            runBlocking { core.discover("not an address", null) }
            fail("expected InvalidAddress")
        } catch (e: DiscoverException.InvalidAddress) {
            assertEquals("discover.invalid_address", discoverErrorMessageKey(e))
        }
    }

    @Test
    fun discoveryTellsTheObserverAboutEveryProbe() {
        val core = openCore(NoCredentials())
        val seen = java.util.concurrent.CopyOnWriteArrayList<Pair<DiscoveryProbe, ProbeProgress>>()
        val observer =
            object : DiscoveryObserver {
                override fun onProbe(probe: DiscoveryProbe, state: ProbeProgress) {
                    seen.add(probe to state)
                }
            }
        // A provider-table address needs no network, only the system's name settings to start.
        val found =
            try {
                runBlocking { core.discover("someone@gmail.com", observer) }
            } catch (e: DiscoverException.ResolverUnavailable) {
                assumeTrue("no system name servers on this machine", false)
                throw e
            }
        assertNotNull(found.imap)
        assertEquals(KnownProvider.GMAIL, found.provider)
        assertEquals(DiscoverySource.PROVIDER_TABLE, found.source)
        assertTrue(seen.contains(DiscoveryProbe.PROVIDER_TABLE to ProbeProgress.Found))
    }

    @Test
    fun aKotlinExceptionInACallbackIsNoCredentialNotAPanic() {
        val first = openCore(NoCredentials())
        val id = runBlocking { first.addAccount(deadServer(), "Alice", null, null, typed()).id }
        first.close()
        open.remove(first)

        val thrown = ThrowingCredentials()
        val core = openCore(thrown)
        // The provider throws from `credential` (and then from `signInNeeded`): the stored account
        // waits for a sign-in; nothing crashes and the exception text stays here.
        assertEquals(AccountStatus.NEEDS_SIGN_IN, core.listAccounts()[0].status)
        try {
            runBlocking { core.signedIn(id) }
            fail("expected NoCredential")
        } catch (e: CoreException.NoCredential) {
            assertFalse(e.toString().contains(ThrowingCredentials.SECRET_TEXT))
        }
        assertTrue(thrown.asked.get() >= 2)
        // The core is still usable (the sign-in state wins over offline on the chip).
        core.setNetworkAvailable(false)
        assertEquals(AccountStatus.NEEDS_SIGN_IN, core.listAccounts()[0].status)
        assertTrue(runBlocking { withTimeout(10_000) { core.nextChange(0u) } }.seq > 0u)
    }

    @Test
    fun aCredentialThatIsNotUtf8IsNoCredential() {
        val bad = { FfiCredential.password(byteArrayOf(0xff.toByte(), 0xfe.toByte(), 0x80.toByte())) }
        val provider = MemoryCredentials()
        val core = openCore(provider)
        // `addAccount` refuses it with a typed error and adds nothing.
        try {
            runBlocking { core.addAccount(deadServer(), "Alice", null, null, bad()) }
            fail("expected NoCredential")
        } catch (e: CoreException.NoCredential) {
            // expected
        }
        assertTrue(core.listAccounts().isEmpty())

        // Stored under an account's id it is "no credential" at sign-in too.
        val id = runBlocking { core.addAccount(deadServer(), "Alice", null, null, typed()).id }
        provider.store(id, bad())
        try {
            runBlocking { core.signedIn(id) }
            fail("expected NoCredential")
        } catch (e: CoreException.NoCredential) {
            // expected
        }
        try {
            runBlocking {
                core.verify(deadServer(), FfiCredential.password(byteArrayOf(0xc0.toByte())), null)
            }
            fail("expected WrongPassword")
        } catch (e: VerifyFailure.WrongPassword) {
            // expected
        }
    }

    @Test
    fun anObserverThatThrowsDoesNotStopDiscovery() {
        val core = openCore(NoCredentials())
        val calls = java.util.concurrent.atomic.AtomicInteger()
        val observer =
            object : DiscoveryObserver {
                override fun onProbe(probe: DiscoveryProbe, state: ProbeProgress) {
                    calls.incrementAndGet()
                    throw IllegalStateException("observer failed with ${ThrowingCredentials.SECRET_TEXT}")
                }
            }
        val found =
            try {
                runBlocking { core.discover("someone@gmail.com", observer) }
            } catch (e: DiscoverException.ResolverUnavailable) {
                assumeTrue("no system name servers on this machine", false)
                throw e
            }
        assertEquals(KnownProvider.GMAIL, found.provider)
        assertTrue(calls.get() > 0)
    }

    @Test
    fun actionsOnUnknownIdsAreTypedErrorsAndChangeNothing() {
        val core = openCore(NoCredentials())
        for (call in listOf<() -> Unit>(
            { core.markSeen(listOf(42L), true) },
            { core.setFlagged(listOf(42L), true) },
            { core.archive(listOf(42L)) },
            { core.trash(listOf(42L)) },
            { core.moveTo(listOf(42L), 1L) },
            { core.deleteForever(listOf(42L)) },
        )) {
            try {
                call()
                fail("expected NoSuchMessage")
            } catch (e: CoreException.NoSuchMessage) {
                // expected
            }
        }
        assertFalse(core.undo(99u))
        core.setUndoWindowMs(3_000L)
        try {
            core.syncNow(7L)
            fail("expected NoSuchAccount")
        } catch (e: CoreException.NoSuchAccount) {
            // expected
        }
    }

    @Test
    fun noPasswordAppearsInAnyErrorOrReturnedValue() {
        val provider = MemoryCredentials()
        val core = openCore(provider)
        val seen = mutableListOf<String>()
        fun note(what: Any?) {
            seen.add(what.toString())
            if (what is Throwable) seen.add(what.message.orEmpty())
        }

        // `FfiCredential` is an object, so it prints no secret (see CredentialPrintingTest); the
        // app (T11) must still never log or print one.
        try {
            runBlocking {
                core.verify(deadServer(), FfiCredential.password(password.toByteArray()), null)
            }
        } catch (e: VerifyFailure) {
            note(e)
            note(verifyFailureMessageKey(e))
            note(verifyFailureDefaultMessage(e))
        }
        val id = runBlocking { core.addAccount(deadServer(), "Alice", null, null, typed()).id }
        note(core.listAccounts())
        note(core.listFolders(id))
        note(core.listMessages(MessageScope.UnifiedInbox, null, 50u))
        note(runBlocking { withTimeout(10_000) { core.nextChange(0u) } })
        try {
            core.archive(listOf(1L))
        } catch (e: CoreException) {
            note(e)
        }
        try {
            core.moveTo(listOf(1L), 5L)
        } catch (e: CoreException) {
            note(e)
        }
        // The database file holds no secret either.
        core.close()
        open.remove(core)
        for (name in listOf("croft.db", "croft.db-wal", "croft.db-shm")) {
            val f = File(dir, name)
            if (f.exists()) seen.add(String(f.readBytes(), Charsets.ISO_8859_1))
        }
        for (text in seen) assertFalse(text.contains(password))
    }
}
