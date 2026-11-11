// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.accounts.AccountColour
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.AccountPrefsJson
import com.example.croft.mail.accounts.CoreAccountOps
import com.example.croft.mail.accounts.Diagnostics
import com.example.croft.mail.accounts.RemovalPlan
import java.io.File
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/**
 * The account screens' logic (S44, S45, S60 and "Try with sample mail") over the **real Rust core** on the JVM: the sample mailbox
 * with the network off, its rows saying they are not saved to a server, removing an account (what the confirmation lists, the order of
 * secret and rows, ids never reused), "Erase all Croft data", and the diagnostics text. Made-up names and `example.org` addresses.
 */
class AccountsTest {
    private val dir: File = Files.createTempDirectory("croft-accounts-test").toFile()
    private val provider = MemoryCredentials()
    private val core: Core = Core.open(File(dir, "croft.db").path, null, provider)
    private val repo = CoreMessageRepository(core)

    /** What happened, in order: "secret:<id>" when an account's saved password was deleted, "core:<id>" when its rows were. */
    private val events = CopyOnWriteArrayList<String>()
    private val secrets = HashSet<Long>()
    private var clearedLocalFiles = false

    private val ops = CoreAccountOps(
        core,
        removeAccountAndSecrets = { id -> synchronized(secrets) { secrets.remove(id) }; events += "secret:$id"; core.removeAccount(id); events += "core:$id" },
        clearSecrets = { synchronized(secrets) { secrets.clear() } },
        clearLocalFiles = { clearedLocalFiles = true },
    )

    @After
    fun tearDown() {
        core.close()
        dir.deleteRecursively()
    }

    private fun deadServer(email: String) = FfiDiscovered(
        email = email,
        imap = ServerSettings("127.0.0.1", 1u, SecurityMode.TLS, "alice"),
        smtp = null, jmap = null, source = DiscoverySource.AUTOCONFIG, provider = null, authHint = AuthKind.PASSWORD,
    )

    /** A real (never reachable) account, with a "saved password" in the fake secret set. */
    private fun addReal(email: String = "alice@example.org"): Long {
        val id = runBlocking { core.addAccount(deadServer(email), "Alice", null, FfiCredential.password("made-up-password".toByteArray())) }
        provider.store(id, "made-up-password")
        synchronized(secrets) { secrets += id }
        return id
    }

    private fun accounts() = runBlocking { repo.listAccounts() }

    private fun inbox(account: Long): List<MessageRowData> = runBlocking {
        val folder = repo.listFolders(account).firstOrNull { it.path == "INBOX" } ?: return@runBlocking emptyList()
        repo.listMessages(MessageScope.Folder(account, folder.id), null, 100).rows
    }

    @Test
    fun trySampleMailWorksWithTheNetworkOffAndTheRowsSayTheyAreNotSavedToAServer() {
        // The airplane-mode flag the app uses: the sample mailbox needs no network at all.
        core.setNetworkAvailable(false)
        val id = runBlocking { ops.addSample() }
        val account = accounts().single()
        assertEquals(id, account.id)
        assertTrue("the core marks it as the sample mailbox", account.isDemo)
        assertEquals(AccountState.OK, account.status)
        assertTrue(provider.signInCalls.isEmpty())
        val rows = await("the sample Inbox") { inbox(id).takeIf { it.size >= 10 } }
        assertTrue("every sample row says it is not saved to a server", rows.all { it.localOnly })
        // Messages in it: a thread, and it can be listed over the unified Inbox too.
        assertTrue(rows.any { it.threadCount > 1 })
        val unified = runBlocking { repo.listMessages(MessageScope.UnifiedInbox, null, 100) }.rows
        assertEquals(rows.size, unified.size)
        // A change stays on this device and the row still says so.
        runBlocking { repo.setFlagged(listOf(rows.first().id), true) }
        await("the star") { inbox(id).firstOrNull { it.id == rows.first().id }?.takeIf { it.flagged } }
        assertTrue(inbox(id).all { it.localOnly })
        // Choosing it twice gives the same account.
        assertEquals(id, runBlocking { ops.addSample() })
        assertEquals(1, accounts().size)
    }

    @Test
    fun aRealAccountsRowsDoNotSayTheyAreNotSaved() {
        val real = addReal()
        assertFalse(accounts().single { it.id == real }.isDemo)
        val sample = runBlocking { ops.addSample() }
        await("the sample Inbox") { inbox(sample).takeIf { it.isNotEmpty() } }
        assertTrue(inbox(sample).all { it.localOnly })
        // Listing the real account's folders gives rows with the flag off (there are none: its server does not exist).
        assertTrue(inbox(real).none { it.localOnly })
    }

    @Test
    fun theRemovalConfirmationListsWhatIsDeletedAndRemovingDeletesIt() {
        val sample = runBlocking { ops.addSample() }
        await("the sample Inbox") { inbox(sample).takeIf { it.size >= 10 } }
        val plan = RemovalPlan.of(accounts().single(), runBlocking { repo.listFolders(sample) })
        assertTrue(plan.sample)
        assertFalse("the sample mailbox has no saved password", plan.savedPassword)
        assertTrue(plan.messages >= 10)
        assertTrue(plan.folders >= 5)
        val real = addReal()
        val realPlan = RemovalPlan.of(accounts().single { it.id == real }, runBlocking { repo.listFolders(real) })
        assertTrue("a real account has a saved password to delete", realPlan.savedPassword)

        runBlocking { ops.remove(sample) }
        assertEquals(listOf(real), accounts().map { it.id })
        assertTrue(runBlocking { repo.listFolders(sample) }.none { !it.isVirtual })
        assertTrue(inbox(sample).isEmpty())
        // Its rows are gone for good: a second removal says there is no such account.
        try {
            runBlocking { ops.remove(sample) }
            fail("expected NoSuchAccount")
        } catch (e: CoreException.NoSuchAccount) {
            // fixed category
        }
    }

    @Test
    fun theSavedPasswordIsDeletedBeforeTheRowsAndAnIdIsNeverGivenAgain() {
        val first = addReal()
        runBlocking { ops.remove(first) }
        assertEquals("the secret first, then the core's rows", listOf("secret:$first", "core:$first"), events.toList())
        assertTrue(synchronized(secrets) { first !in secrets })
        assertTrue(accounts().isEmpty())
        val second = addReal("bob@example.org")
        assertNotEquals("ids are never reused, so a leftover secret could never reach another account", first, second)
        assertTrue(second > first)
    }

    @Test
    fun eraseEverythingRemovesEveryAccountEverySecretAndTheLocalFiles() {
        val real1 = addReal()
        val real2 = addReal("bob@example.org")
        val sample = runBlocking { ops.addSample() }
        synchronized(secrets) { secrets += 99L } // a stray secret of an account that is gone
        runBlocking { ops.eraseEverything() }
        assertTrue(accounts().isEmpty())
        assertTrue(synchronized(secrets) { secrets.isEmpty() })
        assertTrue(clearedLocalFiles)
        assertEquals(listOf(real1, real2, sample).sorted().map { "core:$it" }.toSet(), events.filter { it.startsWith("core:") }.toSet())
        // The app can start over: a new account gets a new id.
        assertTrue(addReal("carol@example.org") > maxOf(real1, real2, sample))
    }

    @Test
    fun diagnosticsHoldNoAddressHostFolderNameOrAccountName() {
        val real = addReal("alice.private@mail.secret-host.example.net")
        val sample = runBlocking { ops.addSample() }
        await("the sample Inbox") { inbox(sample).takeIf { it.isNotEmpty() } }
        val list = accounts()
        val text = Diagnostics.build("1.2.3", "15 (API 35)", "6", list)
        assertTrue(text.contains("App version: 1.2.3"))
        assertTrue(text.contains("FFI API version: 6"))
        assertTrue(text.contains("Accounts: 2"))
        assertTrue(text.contains("(sample mail)"))
        val folderNames = list.flatMap { a -> runBlocking { repo.listFolders(a.id) }.map { it.path } }
        val forbidden = listOf("@", "alice", "secret-host", "example.org", "example.net", "Alice", "Robin") + folderNames
        for (word in forbidden) assertFalse("the diagnostics must not contain \"$word\"", text.contains(word, ignoreCase = true))
        assertTrue(real != sample)
    }

    @Test
    fun theAccountPrefsKeepTheOrderNamesAndColoursAndForgetARemovedAccount() {
        val infos = listOf(1L, 2L, 3L).map { AccountInfo(it, "A$it", "a$it@example.org", AccountState.OK) }
        var prefs = AccountPrefs()
        assertEquals(listOf(1L, 2L, 3L), prefs.ordered(infos).map { it.id })
        prefs = prefs.moved(listOf(1L, 2L, 3L), 3L, -1)
        assertEquals(listOf(1L, 3L, 2L), prefs.ordered(infos).map { it.id })
        // Moving past an end does nothing; an account that is not in the order yet follows it.
        assertEquals(prefs, prefs.moved(listOf(1L, 3L, 2L), 1L, -1))
        assertEquals(listOf(1L, 3L, 2L, 4L), prefs.ordered(infos + AccountInfo(4, "A4", "a4@example.org", AccountState.OK)).map { it.id })
        prefs = prefs.withName(2L, "  Work  ").withColour(3L, AccountColour.AMBER)
        assertEquals("Work", prefs.nameOf(infos[1]))
        assertEquals("A1", prefs.nameOf(infos[0]))
        assertEquals("a blank name goes back to the core's label", "A2", prefs.withName(2L, "   ").nameOf(infos[1]))
        val again = AccountPrefsJson.decodeOrDefault(AccountPrefsJson.encode(prefs))
        assertEquals(prefs, again)
        assertEquals(AccountPrefs(), AccountPrefsJson.decodeOrDefault("not json {"))
        assertEquals("a newer file is not read", AccountPrefs(), AccountPrefsJson.decodeOrDefault("""{"version":99,"order":[1]}"""))
        val forgotten = prefs.without(2L)
        assertEquals(listOf(1L, 3L), forgotten.order)
        assertTrue(forgotten.names.isEmpty())
        assertEquals(mapOf(3L to AccountColour.AMBER), forgotten.colours)
    }
}
