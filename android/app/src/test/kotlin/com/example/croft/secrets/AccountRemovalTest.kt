// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import com.example.croft.mail.accounts.CoreAccountOps
import com.example.croft.mail.accounts.SavedSignIn
import java.io.File
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/**
 * What S44, S45 and S60 do to the real core and the real [SecretStore] (an in-memory wrapping key, the host library through JNA): removing
 * an account through the app's [CoreAccountOps] deletes its rows and its saved password, "Erase all Croft data" leaves no account and no
 * secret, and the sample mailbox never has a password. The accounts' servers are a port nobody listens on; made-up names.
 */
class AccountRemovalTest {
    private lateinit var dir: File
    private lateinit var store: SecretStore
    private val signIns = SignInNeeded()
    private val open = mutableListOf<Core>()

    @Before
    fun setUp() {
        dir = createTempDirectory("croft-removal-test").toFile()
        store = SecretStore(File(dir, "secrets"), AesGcmSecretBox(MemoryKeySource()))
    }

    @After
    fun tearDown() {
        open.forEach { it.close() }
        dir.deleteRecursively()
    }

    private fun core(): Core = Core.open(File(dir, "croft.db").path, null, SecretStoreCredentials(store, signIns)).also { open += it }

    private fun deadServer(email: String) = FfiDiscovered(
        email = email, imap = ServerSettings("127.0.0.1", 1u, SecurityMode.TLS, "alice"),
        smtp = null, jmap = null, source = DiscoverySource.AUTOCONFIG, provider = null, authHint = AuthKind.PASSWORD,
    )

    private fun accountSecrets() = AccountSecrets(store, signIns)

    private fun add(core: Core, email: String): Long =
        runBlocking { accountSecrets().addPasswordAccount(core, deadServer(email), "Alice", null, null, Fixture.PASSWORD.toByteArray()) }

    private fun ops(core: Core, files: MutableList<String> = mutableListOf()) = CoreAccountOps(
        core,
        removeAccountAndSecrets = { id -> accountSecrets().removeAccount(core, id) },
        savedSignIn = { id -> if (store.has(id, SecretKind.PASSWORD)) SavedSignIn.PASSWORD else SavedSignIn.NONE },
        closeCore = { core.close() },
        eraseStorage = { store.clear() },
        clearLocalFiles = { files += "cleared" },
    )

    @Test
    fun removingAnAccountDeletesItsRowsAndItsSavedPassword() {
        val core = core()
        val first = add(core, "alice@example.org")
        val second = add(core, "bob@example.org")
        assertTrue(store.has(first, SecretKind.PASSWORD) && store.has(second, SecretKind.PASSWORD))
        runBlocking { ops(core).remove(first) }
        assertFalse("the saved password is gone with the account", store.has(first, SecretKind.PASSWORD))
        assertEquals(listOf(second), core.listAccounts().map { it.id })
        assertTrue("the other account keeps its password", store.has(second, SecretKind.PASSWORD))
        assertTrue(core.listFolders(first).isEmpty())
    }

    @Test
    fun theSampleMailboxHasNoPasswordToDeleteAndRemovesLikeAnyAccount() {
        val core = core()
        val sample = runBlocking { ops(core).addSample() }
        assertFalse("no secret is ever stored for the sample mailbox", store.hasAny())
        assertTrue(core.listAccounts().single().isDemo)
        runBlocking { ops(core).remove(sample) }
        assertTrue(core.listAccounts().isEmpty())
    }

    @Test
    fun eraseEverythingLeavesNoAccountAndNoSecret() {
        val core = core()
        val a = add(core, "alice@example.org")
        add(core, "bob@example.org")
        runBlocking { ops(core).addSample() }
        store.put(404, SecretKind.OAUTH_REFRESH, "stray".toByteArray()) // a leftover of an account that is gone
        signIns.mark(a)
        val files = mutableListOf<String>()
        runBlocking { ops(core, files).eraseEverything() }
        assertFalse(store.hasAny())
        assertEquals(listOf("cleared"), files)
        assertTrue(signIns.accounts.value.isEmpty())
        // The core was closed with no account left in it; opened again on the same file it lists none, and ids are not reused.
        open.remove(core)
        val again = core()
        assertTrue(again.listAccounts().isEmpty())
        assertTrue(add(again, "carol@example.org") > a)
    }
}
