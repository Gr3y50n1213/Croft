// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import java.security.SecureRandom
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread
import kotlin.io.path.createTempDirectory
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

/**
 * The database key's life: first run, later runs, and what happens when the Keystore key is gone
 * (the data is moved aside, never deleted at once) or merely failing (nothing is touched).
 */
class AppSecretsTest {
    private lateinit var root: File
    private lateinit var dir: File
    private lateinit var db: File
    private lateinit var quarantineDir: File
    private val keys = MemoryKeySource()
    private val sleeps = mutableListOf<Long>()
    private var now = 1_000_000_000L
    private val day = 24L * 60 * 60 * 1000

    @Before
    fun setUp() {
        root = createTempDirectory("croft-appsecrets-test").toFile()
        dir = File(root, "secrets")
        db = File(root, "croft.db")
        quarantineDir = File(root, "croft-quarantine")
    }

    @After
    fun tearDown() {
        root.deleteRecursively()
    }

    private fun secrets(box: SecretBox = AesGcmSecretBox(keys)) = AppSecrets(
        dir, box, keys, db, SecureRandom(), listOf(100L, 500L, 2000L), { sleeps += it },
        Quarantine(quarantineDir, { now }),
    )

    private fun keyBytes(s: AppSecrets): ByteArray = s.databaseKey().use { k -> k.withBytes { it.copyOf() } }

    private fun dbFiles() = listOf("", "-wal", "-shm", "-journal").map { File(db.path + it) }

    private fun makeDatabaseFiles() {
        dbFiles().forEach { it.writeText("not really sqlite ${it.name}") }
    }

    private fun incidents(): List<File> = quarantineDir.listFiles()?.sortedBy { it.name } ?: emptyList()

    private fun failureOf(block: () -> Unit): SecureStorageException.Reason {
        try {
            block()
        } catch (e: SecureStorageException) {
            return e.reason
        }
        fail("expected a SecureStorageException")
        error("unreachable")
    }

    /** A snapshot of every file under [root] with its bytes: proves that nothing moved or changed. */
    private fun snapshot(): Map<String, List<Byte>> =
        root.walkTopDown().filter { it.isFile }.associate { it.relativeTo(root).path to it.readBytes().toList() }

    @Test
    fun theFirstRunMakesA32ByteKeyAndKeepsOnlyAWrappedCopy() {
        val s = secrets()
        val key = keyBytes(s)
        assertEquals(32, key.size)
        assertTrue(key.any { it != 0.toByte() })
        // The wrapped key, and the empty lock file; nothing else.
        val files = dir.listFiles()!!.filter { it.name != "lock" }
        assertEquals(listOf("db-key.v1"), files.map { it.name })
        assertFalse(containsBytes(files.single().readBytes(), key))
        assertFalse(s.keyLoss.pending.value)
        assertFalse(quarantineDir.exists())
    }

    @Test
    fun laterRunsUnwrapTheSameKey() {
        val first = keyBytes(secrets())
        makeDatabaseFiles()
        assertArrayEquals(first, keyBytes(secrets()))
        assertTrue(db.exists())
        assertFalse(quarantineDir.exists())
    }

    @Test
    fun aKeyThatIsClosedIsWipedAndTheCopyGivenToTheBlockIsWiped() {
        val key = secrets().databaseKey()
        var leaked: ByteArray? = null
        key.withBytes { leaked = it }
        assertTrue(leaked!!.all { it == 0.toByte() })
        key.close()
    }

    // ---- a definite loss: moved aside, not deleted ----

    @Test
    fun aLostKeystoreKeyMovesTheDatabaseAndTheSecretsAsideAndStartsAsNew() {
        val s = secrets()
        val old = keyBytes(s)
        makeDatabaseFiles()
        s.secrets.put(1, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        val wrapped = File(dir, "db-key.v1").readBytes()
        val dbBytes = dbFiles().map { it.readBytes() }
        keys.lose()

        val fresh = secrets()
        val key = keyBytes(fresh)
        assertEquals(32, key.size)
        assertNotEquals(old.toList(), key.toList())
        dbFiles().forEach { assertFalse(it.exists()) }
        assertFalse(fresh.secrets.hasAny())
        assertTrue(fresh.keyLoss.pending.value)

        // Everything is in one incident folder, byte for byte.
        val incident = incidents().single()
        assertEquals(now.toString(), incident.name)
        assertArrayEquals(wrapped, File(incident, "key/db-key.v1").readBytes())
        dbFiles().forEachIndexed { i, f -> assertArrayEquals(dbBytes[i], File(incident, "database/${f.name}").readBytes()) }
        assertEquals(listOf("acct-1.password.v1"), File(incident, "secrets").list()!!.toList())

        // The new key is the one kept from now on.
        assertArrayEquals(key, keyBytes(secrets()))
        // The marker survives a new process until the screen has shown it.
        assertTrue(secrets().keyLoss.pending.value)
        fresh.keyLoss.acknowledge()
        assertFalse(fresh.keyLoss.pending.value)
        assertFalse(secrets().keyLoss.pending.value)
    }

    @Test
    fun aWrappedKeyThatFailsAuthenticationIsAlsoALossAndIsMovedAside() {
        val s = secrets()
        s.databaseKey().close()
        makeDatabaseFiles()
        val blob = File(dir, "db-key.v1")
        blob.writeBytes(blob.readBytes().also { it[20] = (it[20].toInt() xor 1).toByte() })
        val damaged = blob.readBytes()
        val s2 = secrets()
        s2.databaseKey().close()
        assertFalse(db.exists())
        assertTrue(s2.keyLoss.pending.value)
        assertArrayEquals(damaged, File(incidents().single(), "key/db-key.v1").readBytes())
    }

    @Test
    fun dataWithoutItsWrappedKeyIsALossAndIsMovedAside() {
        makeDatabaseFiles()
        val s = secrets()
        s.databaseKey().close()
        assertFalse(db.exists())
        assertTrue(s.keyLoss.pending.value)
        assertTrue(File(incidents().single(), "database/croft.db").exists())
    }

    @Test
    fun aLossKeepsTheKeystoreEntry() {
        val s = secrets()
        s.databaseKey().close()
        keys.lose()
        secrets().databaseKey().close()
        assertEquals("the alias is kept", 0, keys.discards)
    }

    @Test
    fun aKeyThatCannotSealTheNewKeyIsReplacedOnce() {
        // The entry is itself the dead key: the new key cannot be sealed until it is replaced.
        val box = ScriptedBox(AesGcmSecretBox(keys))
        box.sealFailures += SecretBoxException.Kind.KEY_LOST
        secrets(box).databaseKey().close()
        assertEquals(1, keys.discards)
        assertEquals(2, box.seals)
    }

    @Test
    fun aWrongCallCanBeUndoneFromTheQuarantine() {
        val s = secrets()
        val original = keyBytes(s)
        makeDatabaseFiles()
        s.secrets.put(1, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())

        // A Keystore failure that was wrongly taken for a loss (the real key is still there).
        val box = ScriptedBox(AesGcmSecretBox(keys))
        box.openFailures += SecretBoxException.Kind.KEY_LOST
        val after = secrets(box)
        assertNotEquals(original.toList(), keyBytes(after).toList())
        val incident = incidents().single()

        // Support moves the files back: everything opens again, with the old key.
        File(dir, "db-key.v1").delete()
        File(incident, "key").listFiles()!!.forEach { it.copyTo(File(dir, it.name)) }
        File(incident, "secrets").listFiles()!!.forEach { it.copyTo(File(dir, it.name), overwrite = true) }
        File(incident, "database").listFiles()!!.forEach { it.copyTo(File(root, it.name), overwrite = true) }
        val restored = secrets()
        assertArrayEquals(original, keyBytes(restored))
        assertArrayEquals(Fixture.PASSWORD.toByteArray(), restored.secrets.get(1, SecretKind.PASSWORD))
        assertTrue(db.exists())
    }

    // ---- everything else: nothing is moved or deleted ----

    @Test
    fun aBusyKeystoreIsRetriedWithBackoffThenFailsNotReadyAndNothingChanges() {
        val s = secrets()
        val key = keyBytes(s)
        makeDatabaseFiles()
        s.secrets.put(1, SecretKind.PASSWORD, "x".toByteArray())
        val before = snapshot()

        keys.unavailable = true
        assertEquals(SecureStorageException.Reason.NOT_READY, failureOf { secrets().databaseKey() })
        assertEquals("three retries with backoff", listOf(100L, 500L, 2000L), sleeps)
        assertEquals("nothing moved, nothing deleted, nothing written", before, snapshot())
        assertFalse(quarantineDir.exists())
        assertFalse(secrets().keyLoss.pending.value)

        keys.unavailable = false
        assertArrayEquals(key, keyBytes(secrets()))
    }

    @Test
    fun aKeystoreThatRecoversDuringTheRetriesGivesTheSameKey() {
        val s = secrets()
        val key = keyBytes(s)
        val box = ScriptedBox(AesGcmSecretBox(keys))
        box.openFailures += SecretBoxException.Kind.UNAVAILABLE
        box.openFailures += SecretBoxException.Kind.UNAVAILABLE
        assertArrayEquals(key, keyBytes(secrets(box)))
        assertEquals(listOf(100L, 500L), sleeps)
        assertFalse(quarantineDir.exists())
    }

    @Test
    fun aKeyFromANewerCroftFailsAsNewerDataAndNothingChanges() {
        val s = secrets()
        s.databaseKey().close()
        makeDatabaseFiles()
        s.secrets.put(1, SecretKind.PASSWORD, "x".toByteArray())
        // The real envelope with a higher version byte, as a newer app would have written it.
        val blob = File(dir, "db-key.v1")
        blob.writeBytes(blob.readBytes().also { it[0] = 2 })
        val before = snapshot()

        assertEquals(SecureStorageException.Reason.NEWER_DATA, failureOf { secrets().databaseKey() })
        assertEquals(before, snapshot())
        assertTrue("no retry for it", sleeps.isEmpty())
        assertFalse(quarantineDir.exists())
        assertFalse(secrets().keyLoss.pending.value)
    }

    @Test
    fun eachKindOfFailureHasItsOwnOutcome() {
        data class Case(val kind: SecretBoxException.Kind, val moves: Boolean, val reason: SecureStorageException.Reason?)
        val cases = listOf(
            Case(SecretBoxException.Kind.KEY_LOST, true, null),
            Case(SecretBoxException.Kind.CORRUPT, true, null),
            Case(SecretBoxException.Kind.UNAVAILABLE, false, SecureStorageException.Reason.NOT_READY),
            Case(SecretBoxException.Kind.UNSUPPORTED_VERSION, false, SecureStorageException.Reason.NEWER_DATA),
        )
        for (case in cases) {
            tearDown()
            setUp()
            val s = secrets()
            s.databaseKey().close()
            makeDatabaseFiles()
            val before = snapshot()
            val box = ScriptedBox(AesGcmSecretBox(keys))
            repeat(10) { box.openFailures += case.kind }
            if (case.reason == null) {
                secrets(box).databaseKey().close()
                assertTrue("${case.kind} moves the data", incidents().size == 1 && !db.exists())
            } else {
                assertEquals("${case.kind}", case.reason, failureOf { secrets(box).databaseKey() })
                assertEquals("${case.kind} leaves everything", before, snapshot())
                assertFalse("${case.kind} makes no quarantine", quarantineDir.exists())
            }
        }
    }

    @Test
    fun aFirstRunThatCannotSealFailsNotReadyAndLeavesNothing() {
        val box = ScriptedBox(AesGcmSecretBox(keys))
        repeat(10) { box.sealFailures += SecretBoxException.Kind.UNAVAILABLE }
        assertEquals(SecureStorageException.Reason.NOT_READY, failureOf { secrets(box).databaseKey() })
        assertEquals(listOf(100L, 500L, 2000L), sleeps)
        assertFalse(File(dir, "db-key.v1").exists())
    }

    @Test
    fun ifTheDataCannotBeMovedAsideNothingIsLostAndNoResetIsReported() {
        val s = secrets()
        s.databaseKey().close()
        makeDatabaseFiles()
        keys.lose()
        // A file where the quarantine folder should be: it cannot be created.
        quarantineDir.writeText("in the way")
        val before = snapshot()
        assertEquals(SecureStorageException.Reason.STORAGE_ERROR, failureOf { secrets().databaseKey() })
        assertEquals(before, snapshot())
        assertFalse("no notice for a reset that did not happen", secrets().keyLoss.pending.value)
    }

    // ---- housekeeping: 7 days and 3 later successful starts ----

    private fun loseAndStartAgain(): File {
        val s = secrets()
        s.databaseKey().close()
        makeDatabaseFiles()
        keys.lose()
        secrets().databaseKey().close() // the start that moved the data aside
        return incidents().single()
    }

    @Test
    fun quarantinedDataIsDeletedOnlyAfterSevenDaysAndThreeLaterStarts() {
        val incident = loseAndStartAgain()
        // The start that made the incident does not count for it.
        assertEquals(0, Quarantine(quarantineDir, { now }).startsOf(incident))

        now += 8 * day
        secrets().databaseKey().close() // start 1
        secrets().databaseKey().close() // start 2
        assertTrue("old enough but only 2 starts", incident.exists())
        secrets().databaseKey().close() // start 3
        assertFalse("7 days and 3 starts", incident.exists())
    }

    @Test
    fun threeStartsWithinSevenDaysKeepIt() {
        val incident = loseAndStartAgain()
        now += 6 * day
        repeat(5) { secrets().databaseKey().close() }
        assertTrue(incident.exists())
        assertEquals(5, Quarantine(quarantineDir, { now }).startsOf(incident))
        now += 1 * day + 1
        secrets().databaseKey().close()
        assertFalse(incident.exists())
    }

    @Test
    fun aStartIsCountedOncePerInstanceNoMatterHowOftenTheKeyIsOpened() {
        val incident = loseAndStartAgain()
        val s = secrets()
        repeat(4) { s.databaseKey().close() }
        assertEquals(1, Quarantine(quarantineDir, { now }).startsOf(incident))
    }

    @Test
    fun aFailedStartIsNotACountedStart() {
        val incident = loseAndStartAgain()
        keys.unavailable = true
        failureOf { secrets().databaseKey() }
        assertEquals(0, Quarantine(quarantineDir, { now }).startsOf(incident))
    }

    // ---- races ----

    @Test
    fun twoInstancesRacingOnTheFirstRunMakeOneKey() {
        val slow = ScriptedBox(AesGcmSecretBox(keys)).apply { sealDelayMs = 300 }
        val results = arrayOfNulls<ByteArray>(2)
        val failures = arrayOfNulls<Throwable>(2)
        val ready = CountDownLatch(1)
        val threads = (0..1).map { i ->
            thread {
                try {
                    ready.await()
                    if (i == 1) Thread.sleep(50)
                    results[i] = keyBytes(secrets(slow))
                } catch (e: Throwable) {
                    failures[i] = e
                }
            }
        }
        ready.countDown()
        threads.forEach { it.join(TimeUnit.SECONDS.toMillis(20)) }
        failures.forEach { if (it != null) throw it }
        assertArrayEquals(results[0], results[1])
        assertEquals("one key was sealed, not two", 1, slow.seals)
        assertFalse(quarantineDir.exists())
    }

    @Test
    fun theProcessHoldsOneInstancePerFolder() {
        ProcessSecrets.clearForTests()
        try {
            val make = { secrets() }
            val a = ProcessSecrets.get(dir, db, make)
            val b = ProcessSecrets.get(File(dir.path + "/../" + dir.name), db) { error("must not be made twice") }
            assertSame(a, b)
            try {
                ProcessSecrets.get(dir, File(root, "other.db")) { error("not reached") }
                fail("expected a refusal for a different database file")
            } catch (e: IllegalStateException) {
                // one folder, one database
            }
            val other = ProcessSecrets.get(File(root, "elsewhere"), db) {
                AppSecrets(File(root, "elsewhere"), AesGcmSecretBox(keys), keys, db)
            }
            assertNotSame(a, other)
        } finally {
            ProcessSecrets.clearForTests()
        }
    }
}
