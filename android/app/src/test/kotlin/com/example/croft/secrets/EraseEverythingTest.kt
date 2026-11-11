// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import java.security.SecureRandom
import kotlin.io.path.createTempDirectory
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * "Erase all Croft data" for the secrets and the database (H1 and M3 of the T17c review): with an earlier database in the quarantine, a
 * wrong key-loss call and its restore must not be able to bring the erased accounts back, the database file and its side files are
 * deleted, and so are every secret, every removal mark, the notices and the wrapped key. Removing one account deletes its quarantined
 * secret files too. Made-up secrets and an in-memory wrapping key.
 */
class EraseEverythingTest {
    private lateinit var root: File
    private lateinit var dir: File
    private lateinit var db: File
    private lateinit var quarantineDir: File
    private val keys = MemoryKeySource()
    private var now = 1_000_000_000L

    @Before
    fun setUp() {
        root = createTempDirectory("croft-erase-test").toFile()
        dir = File(root, "croft-secrets")
        db = File(root, "croft.db")
        quarantineDir = File(root, "croft-quarantine")
    }

    @After
    fun tearDown() { root.deleteRecursively() }

    private fun secrets(box: SecretBox = AesGcmSecretBox(keys)) = AppSecrets(
        dir, box, keys, db, SecureRandom(), listOf(100L, 500L, 2000L), { }, Quarantine(quarantineDir, { now }), { now }, { false },
    )

    private fun keyBytes(s: AppSecrets): ByteArray = s.databaseKey().use { k -> k.withBytes { it.copyOf() } }

    private fun dbFiles() = listOf("", "-wal", "-shm", "-journal").map { File(db.path + it) }

    /**
     * A phone that went through a wrongly declared key loss: the first database, one account secret and the first key are in the
     * quarantine; the person then signed in again (a new database, a new account secret, a new key). Returns the first key.
     */
    private fun phoneWithAnEarlierDatabaseInTheQuarantine(): ByteArray {
        val first = secrets()
        val original = keyBytes(first)
        dbFiles().forEach { it.writeText("old database ${it.name}") }
        first.secrets.put(1, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        val box = ScriptedBox(AesGcmSecretBox(keys))
        box.openFailures += SecretBoxException.Kind.KEY_LOST
        keyBytes(secrets(box)) // the loss is declared: everything moves into the quarantine, a new key is made
        assertTrue(quarantineDir.isDirectory)
        db.writeText("new database")
        SecretStore(dir, AesGcmSecretBox(keys)).put(7, SecretKind.PASSWORD, "new-password".toByteArray())
        return original
    }

    @Test
    fun afterAnEraseNothingIsRestoredFromTheQuarantineAtTheNextStart() {
        val original = phoneWithAnEarlierDatabaseInTheQuarantine()
        File(dir, "loss-suspected.v1").writeText("x")
        val running = secrets()
        running.eraseEverything()

        // Nothing of the earlier phone is left: the quarantine, the database, every secret, the notices and the wrapped key.
        assertFalse(quarantineDir.exists())
        assertTrue(dbFiles().none { it.exists() })
        assertFalse(File(dir, "db-key.v1").exists())
        assertFalse(File(dir, "key-lost.flag").exists())
        assertFalse(File(dir, "loss-suspected.v1").exists())
        assertFalse(running.secrets.hasAny())
        assertFalse(running.keyLoss.pending.value)

        // The next start (a new process) is a first run: a new key, nothing restored, nothing offered.
        val next = secrets()
        val key = keyBytes(next)
        assertNotEquals("a new key, not the earlier one", original.toList(), key.toList())
        assertFalse(next.quarantineRestore.available.value)
        assertFalse("no database came back", db.exists())
        assertFalse("no secret came back", next.secrets.hasAny())
        assertFalse(quarantineDir.exists())
        // The same process can also go on: the next key is made at once.
        assertEquals(32, keyBytes(running).size)
    }

    @Test
    fun withoutTheErasedWrappingKeyTheQuarantinedKeyCouldNotBeOpenedEither() {
        phoneWithAnEarlierDatabaseInTheQuarantine()
        val creates = keys.creates
        secrets().eraseEverything()
        assertTrue("the wrapping key was discarded", keys.discards >= 1)
        // (A copy of a wrapped file taken before the erase cannot be opened: the wrapping key of the next start is a new one.)
        keyBytes(secrets())
        assertEquals(creates + 1, keys.creates)
    }

    @Test
    fun removingOneAccountDeletesItsQuarantinedSecretFilesInEveryIncidentAndLeavesOthers() {
        phoneWithAnEarlierDatabaseInTheQuarantine()
        val incident = quarantineDir.listFiles()!!.single()
        // Account 1's wrapped secret is in the incident; add one more file of account 12 (a prefix look-alike) and one under superseded/.
        File(incident, "secrets/acct-12.password.v1").writeText("other account")
        File(incident, "superseded/secrets").mkdirs()
        File(incident, "superseded/secrets/acct-1.oauth_refresh.v1").writeText("superseded copy")
        assertTrue(File(incident, "secrets/acct-1.password.v1").exists())

        secrets().purgeQuarantinedSecrets(1)

        assertFalse(File(incident, "secrets/acct-1.password.v1").exists())
        assertFalse(File(incident, "superseded/secrets/acct-1.oauth_refresh.v1").exists())
        assertTrue("account 12 is not account 1", File(incident, "secrets/acct-12.password.v1").exists())
        assertTrue("the rest of the incident is untouched", File(incident, "database/croft.db").exists())
    }

    @Test
    fun theAccountRemovalPathPurgesTheQuarantinedCopyThroughAccountSecrets() {
        phoneWithAnEarlierDatabaseInTheQuarantine()
        val incident = quarantineDir.listFiles()!!.single()
        val s = secrets()
        s.secrets.put(1, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        val accounts = AccountSecrets(s.secrets, SignInNeeded(), purgeQuarantined = s::purgeQuarantinedSecrets)
        accounts.finishRemoval(1) // what `removeAccount` does after the core let the account go
        assertFalse(s.secrets.has(1, SecretKind.PASSWORD))
        assertFalse(File(incident, "secrets/acct-1.password.v1").exists())
        assertArrayEquals("another account's live secret is untouched", "new-password".toByteArray(), s.secrets.get(7, SecretKind.PASSWORD))
    }
}
