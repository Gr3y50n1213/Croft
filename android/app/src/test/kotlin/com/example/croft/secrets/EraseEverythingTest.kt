// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import com.example.croft.mail.accounts.AccountSweep
import com.example.croft.mail.accounts.AppFiles
import com.example.croft.mail.accounts.CoreAccountOps
import com.example.croft.mail.accounts.SavedSignIn
import java.io.File
import java.security.SecureRandom
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/**
 * "Erase all Croft data" for the secrets and the database (H1, M3, N1 and N2 of the T17c reviews). A phone that went through a wrong
 * key-loss call has an earlier database, an account secret and a wrapped key in the quarantine; the person then signed in again. An
 * erase must not leave anything a start would restore, **whatever step the app is killed at**, and it must not discard the Keystore key
 * while data survives. Real core (the host library), real secret store and quarantine, an in-memory wrapping key, made-up secrets.
 */
class EraseEverythingTest {
    private lateinit var base: File
    private val open = mutableListOf<Core>()

    @Before
    fun setUp() { base = createTempDirectory("croft-erase-test").toFile() }

    @After
    fun tearDown() {
        open.forEach { try { it.close() } catch (e: Exception) { /* already closed */ } }
        base.deleteRecursively()
    }

    /** A thrown "kill": the app dies at this point of the erase. */
    private class Killed : RuntimeException()

    /** One phone: the no-backup folder [root] with its secrets, database and quarantine. */
    private inner class Phone(name: String) {
        val root = File(base, name).also { it.mkdirs() }
        val dir = File(root, "croft-secrets")
        val db = File(root, "croft.db")
        val quarantineDir = File(root, "croft-quarantine")
        val keys = MemoryKeySource()
        val signIns = SignInNeeded()
        private var now = 1_000_000_000L

        fun secrets(box: SecretBox = AesGcmSecretBox(keys)) = AppSecrets(
            dir, box, keys, db, SecureRandom(), listOf(100L, 500L, 2000L), { }, Quarantine(quarantineDir, { now }), { now }, { false },
        )

        fun dbFiles() = listOf("", "-wal", "-shm", "-journal").map { File(db.path + it) }

        /** The wrong call and the sign-in after it: an earlier database (a text file) and its account 1 are in the quarantine. */
        fun withAnEarlierDatabaseInTheQuarantine(): AppSecrets {
            val first = secrets()
            first.databaseKey().close()
            dbFiles().forEach { it.writeText("old database ${it.name}") }
            first.secrets.put(1, SecretKind.PASSWORD, OLD_PASSWORD.toByteArray())
            val box = ScriptedBox(AesGcmSecretBox(keys))
            box.openFailures += SecretBoxException.Kind.KEY_LOST
            secrets(box).databaseKey().close() // the loss is declared: everything moves into the quarantine, a new key is made
            assertTrue(quarantineDir.isDirectory)
            return secrets()
        }

        fun core(s: AppSecrets): Core =
            Core.open(db.path, null, SecretStoreCredentials(s.secrets, signIns)).also { open += it }

        fun addAccount(core: Core, s: AppSecrets, email: String): Long = runBlocking {
            AccountSecrets(s.secrets, signIns).addPasswordAccount(core, deadServer(email), "Alice", null, null, NEW_PASSWORD.toByteArray())
        }

        fun ops(core: Core, s: AppSecrets, killAt: Int = -1, endCompanion: suspend () -> Unit = {}): CoreAccountOps {
            var step = 0
            fun point() { if (step++ == killAt) throw Killed() }
            val accounts = AccountSecrets(s.secrets, signIns)
            return CoreAccountOps(
                core,
                removeAccountAndSecrets = { id -> point(); accounts.removeAccount(core, id) },
                savedSignIn = { SavedSignIn.PASSWORD },
                eraseQuarantine = { point(); s.eraseQuarantine() },
                closeCore = { point(); core.close() },
                eraseStorage = { point(); s.eraseEverything() },
                clearLocalFiles = { point(); AppFiles.eraseMailFiles(root) },
                endCompanion = endCompanion,
            )
        }
    }

    private fun deadServer(email: String) = FfiDiscovered(
        email = email, imap = ServerSettings("127.0.0.1", 1u, SecurityMode.TLS, "alice"),
        smtp = null, jmap = null, source = DiscoverySource.AUTOCONFIG, provider = null, authHint = AuthKind.PASSWORD,
    )

    /**
     * The erase is killed at step [killAt] of `CoreAccountOps.eraseEverything` (0: the quarantine's deletion, 1: the account removal, 2:
     * closing the core, 3: erasing the storage, 4: deleting Croft's other files; -1: not killed). Then a new process starts.
     */
    private fun eraseKilledAt(killAt: Int): Pair<Phone, AppSecrets> {
        val phone = Phone("kill-$killAt")
        val s = phone.withAnEarlierDatabaseInTheQuarantine()
        val core = phone.core(s)
        phone.addAccount(core, s, "alice@example.org")
        try {
            runBlocking { phone.ops(core, s, killAt).eraseEverything() }
            if (killAt >= 0) fail("expected the kill at step $killAt")
        } catch (e: Killed) {
            // The app died here. (A real kill closes the core with it.)
        }
        try { core.close() } catch (e: Exception) { /* already closed */ }
        val restarted = phone.secrets()
        restarted.databaseKey().close() // the next start
        return phone to restarted
    }

    @Test
    fun aKillAtAnyStepAfterTheQuarantineWentLeavesNothingToRestore() {
        for (killAt in 1..4) {
            val (phone, next) = eraseKilledAt(killAt)
            val label = "killed at step $killAt"
            assertFalse("$label: no restore is offered", next.quarantineRestore.available.value)
            assertFalse("$label: the quarantine is gone", phone.quarantineDir.exists())
            val dbText = if (phone.db.isFile) phone.db.readBytes().take(12).toByteArray().toString(Charsets.ISO_8859_1) else ""
            assertNotEquals("$label: the earlier database did not come back", "old database", dbText)
            assertNotEquals(
                "$label: the earlier account's password did not come back",
                OLD_PASSWORD, next.secrets.get(1, SecretKind.PASSWORD)?.toString(Charsets.UTF_8),
            )
        }
    }

    @Test
    fun anEraseThatNeverStartedChangesNothing() {
        val (phone, next) = eraseKilledAt(0)
        assertTrue("the quarantine is still there (the kill came before its deletion)", phone.quarantineDir.isDirectory)
        // The live data is intact, so the earlier database is only offered, never put back behind the person's back.
        assertTrue(next.quarantineRestore.available.value)
        assertTrue(phone.db.isFile)
    }

    @Test
    fun anEraseSignsTheCompanionOutAndDeletesItsSecretEvenWhenItCannotBeReached() {
        for (reachable in listOf(true, false)) {
            val phone = Phone("companion-$reachable")
            val s = phone.secrets()
            s.databaseKey().close()
            val core = phone.core(s)
            s.secrets.putApp(AppSecretKind.COMPANION, "made-up companion session".toByteArray())
            var signedOut = false
            runBlocking {
                phone.ops(core, s, endCompanion = {
                    if (!reachable) throw java.io.IOException("made-up: unreachable")
                    signedOut = true
                }).eraseEverything()
            }
            assertEquals(reachable, signedOut)
            assertFalse("the Companion secret is gone ($reachable)", phone.secrets().secrets.hasAny())
        }
    }

    @Test
    fun aCompleteEraseLeavesNoCroftFileAndTheNextStartIsAFreshInstall() {
        val phone = Phone("complete")
        val s = phone.withAnEarlierDatabaseInTheQuarantine()
        val core = phone.core(s)
        phone.addAccount(core, s, "alice@example.org")
        val originalKeyFiles = File(phone.dir, "db-key.v1").readBytes().toList()
        runBlocking { phone.ops(core, s).eraseEverything() }
        assertEquals("no Croft file is left in the no-backup folder", emptyList<String>(), phone.root.list()!!.filter { it.startsWith("croft") })
        assertTrue(phone.keys.discards >= 1)

        val next = phone.secrets()
        val key = next.databaseKey().use { k -> k.withBytes { it.copyOf() } }
        assertEquals(32, key.size)
        assertFalse(next.quarantineRestore.available.value)
        assertFalse("no database came back", phone.db.exists())
        assertFalse("no secret came back", next.secrets.hasAny())
        assertNotEquals("a new wrapped key", originalKeyFiles, File(phone.dir, "db-key.v1").readBytes().toList())
    }

    @Test
    fun aDeleteThatFailsStopsTheEraseBeforeTheKeystoreKeyIsDiscardedAndARetryFinishesIt() {
        val phone = Phone("failing")
        val s = phone.withAnEarlierDatabaseInTheQuarantine()
        // The person signed in again (a live secret; without one the next start would put the earlier database back at once).
        s.secrets.put(5, SecretKind.PASSWORD, NEW_PASSWORD.toByteArray())
        // The database "file" cannot be deleted: a folder with something in it.
        phone.db.mkdirs(); File(phone.db, "x").writeText("x")
        try {
            s.eraseEverything()
            fail("expected STORAGE_ERROR")
        } catch (e: SecureStorageException) {
            assertEquals(SecureStorageException.Reason.STORAGE_ERROR, e.reason)
        }
        assertEquals("the Keystore key was kept: what is still there stays usable", 0, phone.keys.discards)
        assertFalse("the quarantine went first", phone.quarantineDir.exists())
        assertTrue("nothing else was touched after the failure", File(phone.dir, "db-key.v1").exists() && s.secrets.hasAny())
        // The next start still works with the data that is left (the key opens it).
        phone.secrets().databaseKey().close()

        phone.db.deleteRecursively()
        s.eraseEverything()
        assertTrue(phone.keys.discards >= 1)
        assertFalse(s.secrets.hasAny())
        assertFalse(File(phone.dir, "db-key.v1").exists())
    }

    @Test
    fun removingOneAccountLeavesTheQuarantineAloneButEraseDeletesIt() {
        val phone = Phone("single")
        val s = phone.withAnEarlierDatabaseInTheQuarantine()
        val core = phone.core(s)
        val id = phone.addAccount(core, s, "alice@example.org")
        assertEquals("the new account has the same number as the quarantined one", 1L, id)
        val incident = phone.quarantineDir.listFiles()!!.single()
        runBlocking { AccountSecrets(s.secrets, phone.signIns).removeAccount(core, id) }
        assertFalse(s.secrets.has(id, SecretKind.PASSWORD))
        assertTrue(
            "the earlier database's account 1 is another account: its quarantined password stays until the quarantine expires or is erased",
            File(incident, "secrets/acct-1.password.v1").exists(),
        )
        s.eraseQuarantine()
        assertFalse(phone.quarantineDir.exists())
    }

    @Test
    fun theSweepNeverTouchesTheQuarantine() {
        val phone = Phone("sweep")
        val s = phone.withAnEarlierDatabaseInTheQuarantine()
        s.secrets.put(7, SecretKind.PASSWORD, NEW_PASSWORD.toByteArray())
        val incident = phone.quarantineDir.listFiles()!!.single()
        AccountSweep(known = setOf(), secretStore = object : AccountSweep.Secrets {
            override fun storedIds() = s.secrets.accountIds()
            override fun deleteSecrets(id: Long) = s.secrets.removeAccount(id)
        }).secrets()
        assertFalse("the stray live secret is gone", s.secrets.has(7, SecretKind.PASSWORD))
        assertTrue("the quarantined secret of the earlier database is not the sweep's to delete", File(incident, "secrets/acct-1.password.v1").exists())
    }

    private companion object {
        const val OLD_PASSWORD = "old-database-pw-Aa-1"
        const val NEW_PASSWORD = "new-database-pw-Bb-2"
    }
}
