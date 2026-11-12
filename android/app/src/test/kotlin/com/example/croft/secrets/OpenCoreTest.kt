// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import com.example.croft.openCore
import java.io.File
import kotlin.io.path.createTempDirectory
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.CredentialProvider
import uniffi.croft_ffi.FfiCredential

/**
 * The database key from [AppSecrets] through `openCore` into the real Rust core (host library
 * built with SQLCipher): the file on disk is encrypted, reopens with the same key, and a lost
 * Keystore key resets everything instead of crashing.
 */
class OpenCoreTest {
    private lateinit var dir: File
    private lateinit var db: File
    private val keys = MemoryKeySource()
    private val open = mutableListOf<Core>()
    private val none = object : CredentialProvider {
        override fun credential(accountId: Long): FfiCredential? = null

        override fun signInNeeded(accountId: Long) {}

        override fun saveTokens(accountId: Long, secret: ByteArray) { secret.fill(0) }
    }

    @Before
    fun setUp() {
        dir = createTempDirectory("croft-opencore-test").toFile()
        db = File(dir, "croft.db")
    }

    @After
    fun tearDown() {
        open.forEach { it.close() }
        dir.deleteRecursively()
    }

    private fun secrets() = AppSecrets(File(dir, "secrets"), AesGcmSecretBox(keys), keys, db)

    private fun core(s: AppSecrets): Core = openCore(db.path, s, none).also { open += it }

    private fun closeAll() {
        open.forEach { it.close() }
        open.clear()
    }

    @Test
    fun theDatabaseIsEncryptedAndReopensWithTheStoredKey() {
        core(secrets())
        closeAll()
        val header = db.readBytes().copyOf(16)
        assertFalse("plain SQLite header", String(header, Charsets.ISO_8859_1).startsWith("SQLite format 3"))
        assertTrue(core(secrets()).listAccounts().isEmpty())
    }

    @Test
    fun theCoreRefusesTheFileWithAnotherKey() {
        core(secrets())
        closeAll()
        // Pretend the wrapped key file was swapped for another key's: opens fine, but is the wrong key.
        val other = AppSecrets(File(dir, "other"), AesGcmSecretBox(keys), keys, File(dir, "other.db"))
        val wrong = other.databaseKey()
        try {
            wrong.withBytes { Core.open(db.path, it, none) }
            throw AssertionError("expected a CoreException")
        } catch (e: CoreException.Storage) {
            assertEquals(uniffi.croft_ffi.StorageReason.NOT_A_DATABASE, e.reason)
        } finally {
            wrong.close()
        }
    }

    @Test
    fun aLostKeystoreKeyStartsAsNewWithANoticeAndNoCrash() {
        core(secrets())
        closeAll()
        val before = db.readBytes()
        keys.lose()
        val s = secrets()
        val core = core(s)
        assertTrue(core.listAccounts().isEmpty())
        assertTrue(s.keyLoss.pending.value)
        assertTrue(db.exists())
        assertFalse(db.readBytes().contentEquals(before))
        // The old database was moved aside, not deleted.
        val moved = File(dir, "croft-quarantine").listFiles()!!.single()
        assertTrue(File(moved, "database/croft.db").readBytes().contentEquals(before))
        closeAll()
        // And the new key is the one that works from now on.
        assertTrue(core(secrets()).listAccounts().isEmpty())
    }

    private fun appWith(mainThread: Boolean, dirName: String = "secrets") = AppSecrets(
        File(dir, dirName), AesGcmSecretBox(keys), keys, db, java.security.SecureRandom(),
        listOf(100L, 500L, 2000L), {}, Quarantine(File(dir, "croft-quarantine"), { 0L }), { 0L }, { mainThread },
    )

    @Test
    fun openCoreRefusesTheMainThreadBeforeAnythingIsTouched() {
        val s = appWith(mainThread = true)
        try {
            core(s)
            throw AssertionError("expected IllegalStateException")
        } catch (e: IllegalStateException) {
            // the main thread
        }
        assertFalse(File(dir, "secrets/db-key.v1").exists())
        assertFalse(db.exists())
    }

    @Test
    fun aStartCountsOnlyOnceTheCoreHasOpened() {
        // An incident from an earlier loss.
        core(secrets())
        closeAll()
        keys.lose()
        core(secrets())
        closeAll()
        val incident = File(dir, "croft-quarantine").listFiles()!!.single()
        val q = Quarantine(File(dir, "croft-quarantine"), { 0L })
        assertEquals("the start that made the incident does not count", 0, q.startsOf(incident))

        // A start whose core refuses the file: the key was fine, the start still does not count.
        val refused = File(dir, "refused.db")
        refused.writeText("not a database")
        val other = AppSecrets(File(dir, "secrets"), AesGcmSecretBox(keys), keys, refused)
        try {
            openCore(refused.path, other, none).also { open += it }
            throw AssertionError("expected CoreException")
        } catch (e: CoreException) {
            // refused: no count
        }
        assertEquals(0, q.startsOf(incident))

        core(secrets())
        assertEquals("one successful start", 1, q.startsOf(incident))
    }
}
