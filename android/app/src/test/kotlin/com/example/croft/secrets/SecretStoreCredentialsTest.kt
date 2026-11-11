// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import uniffi.croft_ffi.AccountStatus
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/**
 * The Keystore-backed provider (over an in-memory wrapping key) against the real Rust core: the
 * host library through JNA, as in MailSurfaceTest. The account's server is a port nobody listens
 * on, so everything seen here is the core's decision about the credential, not the network.
 */
class SecretStoreCredentialsTest {
    private lateinit var dir: File
    private lateinit var store: SecretStore
    private val signIns = SignInNeeded()
    private val open = mutableListOf<Core>()

    @Before
    fun setUp() {
        dir = createTempDirectory("croft-credentials-test").toFile()
        store = SecretStore(File(dir, "secrets"), AesGcmSecretBox(MemoryKeySource()))
    }

    @After
    fun tearDown() {
        open.forEach { it.close() }
        dir.deleteRecursively()
    }

    private fun provider(userFor: (Long) -> String? = { null }) = SecretStoreCredentials(store, signIns, userFor)

    private fun core(provider: SecretStoreCredentials): Core =
        Core.open(File(dir, "croft.db").path, null, provider).also { open += it }

    private fun deadServer() = FfiDiscovered(
        email = "alice@example.org",
        imap = ServerSettings("127.0.0.1", 1u, SecurityMode.TLS, "alice"),
        smtp = null,
        jmap = null,
        source = DiscoverySource.AUTOCONFIG,
        provider = null,
        authHint = AuthKind.PASSWORD,
    )

    private fun <T> await(check: () -> T?): T {
        val end = System.currentTimeMillis() + 10_000
        while (System.currentTimeMillis() < end) {
            check()?.let { return it }
            Thread.sleep(50)
        }
        throw AssertionError("timed out")
    }

    @Test
    fun aStoredPasswordBecomesACredentialAndTheArrayItWasMadeFromIsNotKept() {
        store.put(5, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        val credential = provider().credential(5)
        assertNotNull(credential)
        assertFalse(leaks(credential.toString(), Fixture.PASSWORD))
        assertArrayEquals(Fixture.PASSWORD.toByteArray(), store.get(5, SecretKind.PASSWORD))
    }

    @Test
    fun noSecretMeansNoCredential() {
        assertNull(provider().credential(5))
        assertTrue(signIns.accounts.value.isEmpty())
    }

    @Test
    fun anOAuthTokenIsUsedWhenTheUserNameIsKnownAndOtherwiseThePassword() {
        store.put(5, SecretKind.OAUTH_ACCESS, Fixture.TOKEN.toByteArray())
        assertNull(provider().credential(5)) // no user name and no password
        assertNotNull(provider { "alice" }.credential(5))
        store.put(5, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        assertNotNull(provider().credential(5)) // falls back to the password
    }

    @Test
    fun aFailingKeystoreReadsAsNoCredentialAndNothingIsThrown() {
        val keys = MemoryKeySource()
        val failing = SecretStore(File(dir, "failing"), AesGcmSecretBox(keys))
        failing.put(5, SecretKind.PASSWORD, "x".toByteArray())
        keys.unavailable = true
        assertNull(SecretStoreCredentials(failing, signIns).credential(5))
        keys.unavailable = false
        keys.lose()
        assertNull(SecretStoreCredentials(failing, signIns).credential(5))
    }

    @Test
    fun signInNeededMarksTheAccount() {
        val p = provider()
        p.signInNeeded(3)
        p.signInNeeded(4)
        assertEquals(setOf(3L, 4L), signIns.accounts.value)
        signIns.clear(3)
        assertEquals(setOf(4L), signIns.accounts.value)
    }

    @Test
    fun theStagedPasswordIsStoredUnderTheNewIdAndOnlyOnce() {
        val p = provider()
        val pw = Fixture.PASSWORD.toByteArray()
        p.withStagedPassword(pw) {
            assertNotNull(p.credential(9))
            // A second unknown account is not given it.
            assertNull(p.credential(10))
        }
        assertArrayEquals(Fixture.PASSWORD.toByteArray(), pw) // the caller's array is untouched
        assertArrayEquals(Fixture.PASSWORD.toByteArray(), store.get(9, SecretKind.PASSWORD))
        assertFalse(store.has(10, SecretKind.PASSWORD))
        // Outside the block nothing is offered.
        assertNull(p.credential(11))
    }

    @Test
    fun theCoreGetsTheStoredPasswordWhenItAddsAndReopensAnAccount() {
        val p = provider()
        val first = core(p)
        val id = p.withStagedPassword(Fixture.PASSWORD.toByteArray()) {
            runBlocking { first.addAccount(deadServer(), "Alice", null) }
        }
        // The core asked for the new account's credential; the answer was stored under its id.
        assertTrue(store.has(id, SecretKind.PASSWORD))
        assertEquals(AccountStatus.OK, first.listAccounts().single().status)
        first.close()
        open.remove(first)

        // A fresh provider over the same store hands it to the core again at open.
        val second = core(provider())
        assertEquals(AccountStatus.OK, second.listAccounts().single().status)
        assertTrue(signIns.accounts.value.isEmpty())
    }

    @Test
    fun aMissingSecretLeadsToNeedsSignInAndTheAccountIsMarked() {
        val p = provider()
        val first = core(p)
        val id = p.withStagedPassword(Fixture.PASSWORD.toByteArray()) {
            runBlocking { first.addAccount(deadServer(), "Alice", null) }
        }
        first.close()
        open.remove(first)

        // The secret is gone (removed, or the keystore was reset): the account asks for a sign-in.
        store.removeAccount(id)
        val second = core(p)
        assertEquals(AccountStatus.NEEDS_SIGN_IN, second.listAccounts().single().status)
        await { signIns.accounts.value.takeIf { it.contains(id) } }

        // The person signs in again: the secret is stored, the mark is cleared, the core is told.
        store.put(id, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        signIns.clear(id)
        runBlocking { second.signedIn(id) }
        assertEquals(AccountStatus.OK, second.listAccounts().single().status)
        assertTrue(signIns.accounts.value.isEmpty())
    }
}
