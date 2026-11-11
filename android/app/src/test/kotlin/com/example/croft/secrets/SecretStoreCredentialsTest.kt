// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import com.example.croft.mail.onboarding.SignInNotSavedException
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Assert.fail
import org.junit.Test
import uniffi.croft_ffi.AccountStatus
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.CredentialProvider
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.ProviderException
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

    private fun core(provider: CredentialProvider): Core =
        Core.open(File(dir, "croft.db").path, null, provider).also { open += it }

    private fun deadServer(email: String = "alice@example.org") = FfiDiscovered(
        email = email,
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
    fun aKeystoreThatCannotAnswerIsNotReadyAndALostKeyReadsAsNoCredential() {
        val keys = MemoryKeySource()
        val failing = SecretStore(File(dir, "failing"), AesGcmSecretBox(keys))
        failing.put(5, SecretKind.PASSWORD, "x".toByteArray())
        // UNAVAILABLE: not "no credential" (that would send the person to sign in again).
        keys.unavailable = true
        try {
            SecretStoreCredentials(failing, signIns).credential(5)
            fail("expected ProviderException.NotReady")
        } catch (e: ProviderException.NotReady) {
            // the core treats it as a transient failure
        }
        assertTrue("sign_in_needed is not ours to call for it", signIns.accounts.value.isEmpty())
        // A loss that is only suspected (Android 8 to 11) is not a sign-in problem either.
        keys.unavailable = false
        keys.lose()
        keys.weakLoss = true
        try {
            SecretStoreCredentials(failing, signIns).credential(5)
            fail("expected ProviderException.NotReady")
        } catch (e: ProviderException.NotReady) {
            // same
        }
        // A definite loss is "no credential": the secret is gone.
        keys.weakLoss = false
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

    /** Records which accounts the core asked about, over the real store-backed provider. */
    private class Recording(private val inner: CredentialProvider) : CredentialProvider {
        val asked = CopyOnWriteArrayList<Long>()

        override fun credential(accountId: Long): FfiCredential? {
            asked.add(accountId)
            return inner.credential(accountId)
        }

        override fun signInNeeded(accountId: Long) = inner.signInNeeded(accountId)
    }

    private fun accounts() = AccountSecrets(store, signIns)

    private fun add(core: Core, password: String = Fixture.PASSWORD, email: String = "alice@example.org"): Long =
        runBlocking { accounts().addPasswordAccount(core, deadServer(email), "Alice", null, null, password.toByteArray()) }

    @Test
    fun aNewAccountIsStoredUnderItsReturnedIdAndTheProviderWasNotAskedAboutIt() {
        val recording = Recording(provider())
        val core = core(recording)
        val id = add(core)
        assertArrayEquals(Fixture.PASSWORD.toByteArray(), store.get(id, SecretKind.PASSWORD))
        assertTrue("the provider is only asked about stored accounts", recording.asked.isEmpty())
        assertEquals(AccountStatus.OK, core.listAccounts().single().status)
    }

    @Test
    fun aNewAccountNeverGetsAnotherAccountsPasswordOrTheOtherWayAround() {
        val recording = Recording(provider())
        val core = core(recording)
        // Two addresses: the same address again would be a sign-in again for the first account.
        val first = add(core, "first-account-pw-Ab-1", "first@example.org")
        // The second account has its own password; nothing is "staged" for it to pick up.
        val second = add(core, "second-account-pw-Cd-2", "second@example.org")
        assertNotEquals(first, second)
        assertArrayEquals("first-account-pw-Ab-1".toByteArray(), store.get(first, SecretKind.PASSWORD))
        assertArrayEquals("second-account-pw-Cd-2".toByteArray(), store.get(second, SecretKind.PASSWORD))
        // An id nobody stored anything for stays unanswered, whatever was added before.
        assertNull(provider().credential(second + 1))
        assertTrue(recording.asked.isEmpty())
    }

    @Test
    fun ifAddAccountFailsNothingIsStored() {
        val core = core(provider())
        val jmapOnly = deadServer().copy(imap = null, jmap = "https://example.org/.well-known/jmap")
        try {
            runBlocking { accounts().addPasswordAccount(core, jmapOnly, "Alice", null, null, Fixture.PASSWORD.toByteArray()) }
            fail("expected CoreException")
        } catch (e: CoreException) {
            // refused by the core
        }
        assertFalse(store.hasAny())
        assertTrue(core.listAccounts().isEmpty())
    }

    @Test
    fun ifTheSecretCannotBeStoredTheAccountIsNotKept() {
        val keys = MemoryKeySource()
        val failing = SecretStore(File(dir, "failing"), AesGcmSecretBox(keys))
        val core = core(SecretStoreCredentials(failing, signIns))
        keys.unavailable = true
        try {
            runBlocking {
                AccountSecrets(failing, signIns)
                    .addPasswordAccount(core, deadServer(), "Alice", null, null, Fixture.PASSWORD.toByteArray())
            }
            fail("expected SecretBoxException")
        } catch (e: SecretBoxException) {
            assertEquals(SecretBoxException.Kind.UNAVAILABLE, e.kind)
        }
        assertTrue(core.listAccounts().isEmpty())
    }

    @Test
    fun signingInAgainForTheSameAddressKeepsTheAccountAndReplacesItsPassword() {
        val recording = Recording(provider())
        val core = core(recording)
        val id = add(core, "old-account-pw-Gh-4")
        // A leftover token of another kind of sign-in, and the mark of a sign-in that was needed.
        store.put(id, SecretKind.OAUTH_REFRESH, "stale".toByteArray())
        signIns.mark(id)

        val again = add(core, "new-account-pw-Ij-5")
        assertEquals("the core updated the account, it did not add one", id, again)
        assertEquals(1, core.listAccounts().size)
        assertArrayEquals("the new password replaced the old one", "new-account-pw-Ij-5".toByteArray(), store.get(id, SecretKind.PASSWORD))
        assertFalse("no token may answer instead of the new password", store.has(id, SecretKind.OAUTH_REFRESH))
        assertTrue("the sign-in-needed mark is cleared", signIns.accounts.value.isEmpty())
        // What the core gets from the store at the next start is the new password.
        val credential = provider().credential(id)
        assertNotNull(credential)
        assertTrue("the provider was never asked about it meanwhile", recording.asked.isEmpty())
    }

    @Test
    fun signingInAgainOnAnotherServerIsRefusedAndStoresNothing() {
        val core = core(provider())
        val id = add(core, "first-account-pw-Kl-6")
        val other = deadServer().copy(imap = ServerSettings("localhost", 1u, SecurityMode.TLS, "alice"))
        try {
            runBlocking { accounts().addPasswordAccount(core, other, "Alice", null, null, "other-account-pw-Mn-7".toByteArray()) }
            fail("expected CoreException.AccountExists")
        } catch (e: CoreException.AccountExists) {
            // "This address is already added"
        }
        assertEquals(1, core.listAccounts().size)
        assertArrayEquals("the old password is untouched", "first-account-pw-Kl-6".toByteArray(), store.get(id, SecretKind.PASSWORD))
    }

    @Test
    fun ifTheNewPasswordCannotBeStoredWhenSigningInAgainTheUpdateIsUndoneAndTheAccountIsKept() {
        val keys = MemoryKeySource()
        val local = SecretStore(File(dir, "again"), AesGcmSecretBox(keys))
        val core = core(SecretStoreCredentials(local, signIns))
        val accounts = AccountSecrets(local, signIns)
        val id = runBlocking { accounts.addPasswordAccount(core, deadServer(), "Alice", null, null, "old-account-pw-Op-8".toByteArray()) }
        keys.unavailable = true
        try {
            runBlocking { accounts.addPasswordAccount(core, deadServer(), "Alice", null, null, "new-account-pw-Qr-9".toByteArray()) }
            fail("expected SignInNotSavedException")
        } catch (e: SignInNotSavedException) {
            // "the account is as it was": the screen says so
        }
        // Unlike a new account, an account that was only signed in again is kept, with its old secret.
        assertEquals(listOf(id), core.listAccounts().map { it.id })
        assertTrue(local.has(id, SecretKind.PASSWORD))
        keys.unavailable = false
        assertArrayEquals("old-account-pw-Op-8".toByteArray(), local.get(id, SecretKind.PASSWORD))
        // The core's update was undone, so the core runs the account with the stored (old) password again and asked the app for it.
        assertEquals(AccountStatus.OK, core.listAccounts().single().status)
    }

    @Test
    fun aPinnedAccountIsNotReplacedOrDroppedWithoutTheAcknowledgement_andNothingIsStored() {
        val core = core(provider())
        val pin = "AB".repeat(32)
        val id = runBlocking { accounts().addPasswordAccount(core, deadServer(), "Alice", pin, null, "first-account-pw-Rs-10".toByteArray()) }
        for (other in listOf("CD".repeat(32), null)) {
            try {
                runBlocking { accounts().addPasswordAccount(core, deadServer(), "Alice", other, null, "other-account-pw-Tu-11".toByteArray()) }
                fail("expected CoreException.CertificateChanged")
            } catch (e: CoreException.CertificateChanged) {
                // the changed-certificate warning comes first
            }
            assertArrayEquals("the old password is untouched", "first-account-pw-Rs-10".toByteArray(), store.get(id, SecretKind.PASSWORD))
        }
        // After the person confirmed the warning the pin and the password are replaced.
        val again = runBlocking { accounts().addPasswordAccount(core, deadServer(), "Alice", "CD".repeat(32), "CD".repeat(32), "new-account-pw-Vw-12".toByteArray()) }
        assertEquals(id, again)
        assertArrayEquals("new-account-pw-Vw-12".toByteArray(), store.get(id, SecretKind.PASSWORD))
        assertEquals(1, core.listAccounts().size)
    }

    @Test
    fun ifTheSecretFileCannotBeWrittenTheAccountIsNotKeptEither() {
        val core = core(provider())
        // The first account will get id 1. A directory where its temporary file must go makes the write fail
        // with an IOException (not a SecretBoxException).
        File(File(dir, "secrets"), "acct-1.password.v1.tmp").mkdirs()
        try {
            add(core)
            fail("expected IOException")
        } catch (e: java.io.IOException) {
            // the write failed
        }
        assertTrue("the account was removed again", core.listAccounts().isEmpty())
        assertFalse(store.has(1, SecretKind.PASSWORD))
    }

    @Test
    fun theCallersPasswordArrayIsLeftAloneAndAddRunsOffTheCallingThread() {
        val core = core(provider())
        val password = Fixture.PASSWORD.toByteArray()
        var thread: Thread? = null
        val accounts = AccountSecrets(store, signIns, kotlinx.coroutines.Dispatchers.IO)
        val id = runBlocking {
            thread = Thread.currentThread()
            accounts.addPasswordAccount(core, deadServer(), "Alice", null, null, password)
        }
        assertArrayEquals("the copy given to the core is wiped, the caller's array is the caller's", Fixture.PASSWORD.toByteArray(), password)
        assertArrayEquals(Fixture.PASSWORD.toByteArray(), store.get(id, SecretKind.PASSWORD))
        assertNotNull(thread)
    }

    @Test
    fun aKeystoreThatIsNotReadyAtOpenDoesNotMarkTheAccountAndTheCoreAsksAgain() {
        val keys = MemoryKeySource()
        val local = SecretStore(File(dir, "notready"), AesGcmSecretBox(keys))
        val first = core(SecretStoreCredentials(local, signIns))
        val id = runBlocking {
            AccountSecrets(local, signIns).addPasswordAccount(first, deadServer(), "Alice", null, null, Fixture.PASSWORD.toByteArray())
        }
        first.close()
        open.remove(first)

        keys.unavailable = true
        val recording = Recording(SecretStoreCredentials(local, signIns))
        val second = core(recording)
        assertEquals(listOf(id), recording.asked.toList())
        assertEquals("not a sign-in problem", AccountStatus.OK, second.listAccounts().single().status)
        // The core asks again by itself after a back-off (a poke wakes the account's task).
        keys.unavailable = false
        await {
            second.syncNow(id)
            recording.asked.size.takeIf { it >= 2 }
        }
        Thread.sleep(300)
        assertEquals(AccountStatus.OK, second.listAccounts().single().status)
        assertTrue("the account was never marked as needing a sign-in", signIns.accounts.value.isEmpty())
    }

    @Test
    fun anIdIsNeverReusedAndALeftoverUnderTheNewIdIsCleared() {
        val core = core(provider())
        add(core, email = "first@example.org")
        val second = add(core, email = "second@example.org")
        runBlocking { accounts().removeAccount(core, second) }
        // A leftover of a crash, under the id the next account will get if ids were reused (2) or
        // the next one (3): both are cleared before the new secret is stored.
        store.put(2, SecretKind.OAUTH_REFRESH, "stale".toByteArray())
        store.put(3, SecretKind.OAUTH_REFRESH, "stale".toByteArray())
        val third = add(core, "third-account-pw-Ef-3", "third@example.org")
        assertEquals("ids are never reused", 3L, third)
        assertFalse(store.has(3, SecretKind.OAUTH_REFRESH))
        assertArrayEquals("third-account-pw-Ef-3".toByteArray(), store.get(3, SecretKind.PASSWORD))
    }

    @Test
    fun removingAnAccountStopsItInTheCoreFirstThenDeletesItsSecretsAndQuarantinedCopiesThenTheMark() {
        val core = core(provider())
        val id = add(core)
        signIns.mark(id)
        // What the removal's last steps see: the core has already let the account go, the removal mark is still there.
        val seen = mutableListOf<String>()
        val removal = AccountSecrets(store, signIns, purgeQuarantined = { purged ->
            seen += "purge:$purged core-has-it=${core.listAccounts().any { it.id == purged }} secret=${store.has(purged, SecretKind.PASSWORD)} mark=${purged in store.removing()}"
        })
        runBlocking { removal.removeAccount(core, id) }
        assertEquals(listOf("purge:$id core-has-it=false secret=false mark=true"), seen)
        assertFalse(store.has(id, SecretKind.PASSWORD))
        assertTrue(core.listAccounts().isEmpty())
        assertTrue(signIns.accounts.value.isEmpty())
        assertTrue("the removal mark is gone at the end", store.removing().isEmpty())
    }

    @Test
    fun aRemovalThatWasCutShortIsFinishedAndAnUnknownIdIsNotAnError() {
        val core = core(provider())
        val id = add(core)
        // The app was killed after the core let the account go and before its secret was deleted: the mark and the secret are left.
        store.markRemoving(id)
        runBlocking { core.removeAccount(id) }
        assertTrue(store.has(id, SecretKind.PASSWORD))
        // Removing it again (the core says there is no such account) finishes the job without an error.
        runBlocking { accounts().removeAccount(core, id) }
        assertFalse(store.has(id, SecretKind.PASSWORD))
        assertTrue(store.removing().isEmpty())
        // An id that never existed: whatever is stored under it goes too.
        store.put(999, SecretKind.PASSWORD, "x".toByteArray())
        runBlocking { accounts().removeAccount(core, 999) }
        assertFalse(store.has(999, SecretKind.PASSWORD))
    }

    @Test
    fun theCoreGetsTheStoredPasswordWhenItReopensAnAccount() {
        val first = core(provider())
        val id = add(first)
        assertEquals(AccountStatus.OK, first.listAccounts().single().status)
        first.close()
        open.remove(first)

        // A fresh provider over the same store hands it to the core again at open.
        val recording = Recording(provider())
        val second = core(recording)
        assertEquals(AccountStatus.OK, second.listAccounts().single().status)
        assertEquals(listOf(id), recording.asked.toList())
        assertTrue(signIns.accounts.value.isEmpty())
    }

    @Test
    fun aMissingSecretLeadsToNeedsSignInAndTheAccountIsMarked() {
        val p = provider()
        val first = core(p)
        val id = add(first)
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
