// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.content.Context
import android.os.Looper
import java.io.File
import java.io.IOException
import java.nio.channels.FileChannel
import java.nio.file.StandardOpenOption
import java.security.SecureRandom
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** The 32-byte database key. Prints as `[redacted]`; [withBytes] wipes its copy after use. */
class DatabaseKey internal constructor(private var bytes: ByteArray) : AutoCloseable {
    /** Runs [block] with a copy of the key and wipes the copy afterwards. */
    fun <R> withBytes(block: (ByteArray) -> R): R {
        val copy = bytes.copyOf()
        try {
            return block(copy)
        } finally {
            copy.fill(0)
        }
    }

    /** Wipes the key held here. */
    override fun close() {
        bytes.fill(0)
    }

    override fun toString(): String = "[redacted]"
}

/**
 * Why the secure storage could not be opened. None of these resets or deletes anything: the
 * caller shows the message and the person can try again (or update the app).
 *
 * The kinds, with the plain text the screen shows (the UI owns the wording and the translations):
 * - [Reason.NOT_READY]: "Secure storage is not ready; try again." The Keystore failed in a way
 *   that may pass. Retried with backoff before this was thrown.
 * - [Reason.NEWER_DATA]: "This data was written by a newer Croft." Update the app.
 * - [Reason.STORAGE_ERROR]: "Croft could not move its old data aside; try again." The files of a
 *   lost key could not be moved into the quarantine folder (disk full, permissions).
 *
 * The exception carries only the reason: no text, no cause.
 */
class SecureStorageException(val reason: Reason) : Exception(reason.name) {
    enum class Reason { NOT_READY, NEWER_DATA, STORAGE_ERROR }
}

/**
 * Tells the first screen that the database and the account secrets were moved aside because the
 * keystore key was gone, so it can say so in plain words (a later task draws that). Kept in a
 * small marker file, so it survives the process being killed before the screen was shown.
 */
class KeyLossNotice(private val marker: File) {
    private val state = MutableStateFlow(marker.exists())

    /** True from the reset until [acknowledge]. */
    val pending: StateFlow<Boolean> get() = state

    internal fun raise() {
        writeAtomically(marker, ByteArray(0))
        state.value = true
    }

    /** The screen has shown the message. */
    fun acknowledge() {
        marker.delete()
        state.value = false
    }

    override fun toString(): String = "[redacted]"
}

/** Whether the calling thread is the app's main thread. Injectable so JVM tests can say yes. */
internal fun interface MainThreadCheck {
    fun isMainThread(): Boolean

    companion object {
        /** The real check. A JVM without Android's `Looper` (plain unit tests) is never the main thread. */
        val ANDROID = MainThreadCheck {
            try {
                val looper = Looper.myLooper()
                looper != null && looper == Looper.getMainLooper()
            } catch (e: RuntimeException) {
                false
            }
        }
    }
}

/**
 * A quarantined database and its secrets that can be put back: the wrapped database key in the
 * quarantine opens with the **current** wrapping key, so the loss that moved the data aside was a
 * false alarm (a Keystore that failed, not a key that was gone).
 *
 * At the first [AppSecrets.databaseKey] of a process `AppSecrets` looks for such an incident (the
 * newest one). If nothing new was written since the reset (no account secret exists, which is the
 * case until a person signs in again), it puts the old database, secrets and key back by itself and
 * clears [KeyLossNotice]. If there is new data, it does nothing to it and sets [available] so a
 * screen can offer "restore my earlier mail and accounts". [restore] does that: the new data moves
 * into the incident's `superseded/` folder (kept like any quarantine, not deleted), so the choice
 * is reversible too. [dismiss] hides the offer for good. Call [restore] with the core closed and
 * off the main thread, then open the core again.
 */
class QuarantineRestore internal constructor(private val owner: AppSecrets) {
    private val state = MutableStateFlow(false)

    /** True when an earlier database can be restored and was not restored automatically. Set by the first `databaseKey()` of a process. */
    val available: StateFlow<Boolean> get() = state

    internal fun set(value: Boolean) {
        state.value = value
    }

    /**
     * Puts the quarantined database, secrets and key back; the current ones move into the
     * incident. Throws [SecureStorageException] (STORAGE_ERROR) when files cannot be moved. Does
     * nothing when [available] is false. The core must be closed.
     */
    fun restore() = owner.restoreFromQuarantine()

    /** The person does not want the earlier data: [available] turns false and stays so. */
    fun dismiss() = owner.dismissQuarantineOffer()

    override fun toString(): String = "[redacted]"
}

/**
 * The app's secrets: the database key, the account secrets and the key-loss notice, all kept in
 * [dir] (the app's `noBackupFilesDir/croft-secrets`) and wrapped by [box]. Use one instance per
 * process (`createAppSecrets` does); [databaseKey] also takes a file lock, so a second process or
 * a second instance cannot make a second key while the first is being made.
 *
 * **Threads.** [databaseKey] (and `openCore`, which calls it) can wait on the Keystore for several
 * seconds: up to 0.6 s per check and three retries with back-off (about 3 s), and a first key can
 * take longer on a busy secure element. It throws [IllegalStateException] on the main thread, so
 * the mistake shows up in the first test run, not as an ANR in the field. The waits happen with no
 * monitor held (the key source and [SecretStore] release theirs before sleeping, and this class
 * releases its process lock between retries).
 *
 * [databaseFile] is the SQLite file the key belongs to. It is only touched when the key is
 * lost: then it (and its `-wal`, `-shm` and `-journal` files), every account secret and the
 * wrapped key are **moved**, not deleted, into `croft-quarantine/<timestamp>/` next to [dir]
 * (see [Quarantine]), a new key is made and [keyLoss] is raised. The mail is synced again from
 * the servers; the person signs in again. Quarantined data is deleted after 7 days and 3 later
 * successful starts (see [recordSuccessfulStart]: a start counts once `Core.open` has succeeded).
 * The Keystore entry is kept, so quarantined data can still be opened if the loss was a wrong
 * call: [quarantineRestore] then puts it back (automatically when nothing new was written).
 *
 * What counts as lost: the wrapped key's envelope is damaged or fails authentication
 * ([SecretBoxException.Kind.CORRUPT]); the Keystore says the key is definitely gone (KEY_LOST,
 * see [KeystoreErrors] and [KeystoreKeySource]); or data exists without its wrapped key. Weak
 * evidence of a lost key ([SecretBoxException.weak]: Android 8 to 11, or a keystore in which the
 * key and its canary both look absent) is acted on only after it has been seen at 3 app starts
 * over at least an hour ([LossConfirmation]); until then the open fails with NOT_READY and nothing
 * moves. A Keystore that merely fails (UNAVAILABLE) is retried with backoff and then reported as
 * [SecureStorageException.Reason.NOT_READY]; a blob from a newer Croft (UNSUPPORTED_VERSION) as
 * NEWER_DATA. Neither moves or deletes anything.
 *
 * **A wrapping key is created only when no wrapped file exists** (the first run: no `db-key.v1`,
 * no account secret, no database, no quarantine) **or in the reset after a declared loss**, when
 * everything wrapped has been moved into the quarantine. Sealing never creates it.
 */
class AppSecrets internal constructor(
    private val dir: File,
    private val box: SecretBox,
    private val keys: WrappingKeySource,
    private val databaseFile: File,
    private val random: SecureRandom,
    private val retryDelaysMs: List<Long>,
    private val sleep: (Long) -> Unit,
    private val quarantine: Quarantine,
    private val clock: () -> Long,
    private val mainThread: MainThreadCheck,
) {
    constructor(
        dir: File,
        box: SecretBox,
        keys: WrappingKeySource,
        databaseFile: File,
        random: SecureRandom = SecureRandom(),
    ) : this(
        dir, box, keys, databaseFile, random, DEFAULT_RETRY_DELAYS_MS, Thread::sleep,
        Quarantine(File(dir.absoluteFile.parentFile, Quarantine.FOLDER_NAME), System::currentTimeMillis),
        System::currentTimeMillis, MainThreadCheck.ANDROID,
    )

    init {
        dir.mkdirs()
    }

    /** Account passwords and tokens. */
    val secrets = SecretStore(dir, box)

    /** Raised when the keystore key was gone and everything was moved aside. */
    val keyLoss = KeyLossNotice(File(dir, KEY_LOST_MARKER))

    /** Earlier data that can be put back after a false alarm (see the class text). */
    val quarantineRestore = QuarantineRestore(this)

    private val keyFile = File(dir, "db-key.v1")
    private val lockFile = File(dir, LOCK_FILE)
    private val suspicion = LossConfirmation(File(dir, SUSPICION_FILE), clock)
    private var startCounted = false
    private var restoreChecked = false
    private var incidentThisStart: File? = null
    private var offered: File? = null

    /** What one pass of [databaseKey] came to. */
    private sealed interface Attempt {
        class Done(val key: DatabaseKey) : Attempt

        object Retry : Attempt
    }

    /**
     * Throws [IllegalStateException] on the main thread. `openCore` calls it first, so that a
     * call from the wrong thread fails before anything is read.
     */
    internal fun assertNotMainThread() {
        check(!mainThread.isMainThread()) {
            "Secure storage waits on the Keystore: call it off the main thread (for example Dispatchers.IO)"
        }
    }

    /**
     * The database key: made on first run, opened from its wrapped file afterwards. A key that is
     * lost is handled as described in the class text. Throws [SecureStorageException] when the
     * storage cannot be used now (nothing is moved or deleted then), and [IllegalStateException] on
     * the main thread. A start is **not** counted here: `openCore` counts it after `Core.open`
     * succeeded (see [recordSuccessfulStart]). Wipe or [DatabaseKey.close] the result.
     */
    fun databaseKey(): DatabaseKey {
        assertNotMainThread()
        var attempt = 0
        while (true) {
            // The process lock is held for one pass only; the back-off sleeps without it.
            when (val result = withLock { onePass() }) {
                is Attempt.Done -> return result.key
                Attempt.Retry -> {
                    if (attempt >= retryDelaysMs.size) throw notReady()
                    sleep(retryDelaysMs[attempt++])
                }
            }
        }
    }

    /**
     * Counts this process as one successful start for the quarantine's housekeeping (once per
     * process; the incident made during this very start does not count). `openCore` calls it after
     * `Core.open` succeeded, so a start that failed to open the core never counts. Never throws.
     */
    internal fun recordSuccessfulStart() {
        try {
            withLock {
                if (!startCounted) {
                    startCounted = true
                    quarantine.recordSuccessfulStart(except = incidentThisStart)
                }
            }
        } catch (e: SecureStorageException) {
            // Housekeeping only: tried again at the next start.
        }
    }

    private fun onePass(): Attempt {
        try {
            var key = obtainKey()
            if (!restoreChecked) {
                val restored = try {
                    checkQuarantine()
                } catch (e: IOException) {
                    key.close()
                    throw e
                } catch (e: SecretBoxException) {
                    // The check could not be made now (Keystore busy): the key is fine, try the check at the next call.
                    false
                }
                if (restored) {
                    key.close()
                    key = obtainKey()
                }
            }
            return Attempt.Done(key)
        } catch (e: SecretBoxException) {
            return when (e.kind) {
                SecretBoxException.Kind.UNSUPPORTED_VERSION ->
                    throw SecureStorageException(SecureStorageException.Reason.NEWER_DATA)
                SecretBoxException.Kind.UNAVAILABLE -> Attempt.Retry
                // A loss that was not confirmed (weak, or one that came while sealing): not now.
                SecretBoxException.Kind.KEY_LOST, SecretBoxException.Kind.CORRUPT -> throw notReady()
            }
        }
    }

    /** The key, however it has to be made: stored, first run or the reset after a loss. */
    private fun obtainKey(): DatabaseKey {
        if (keyFile.exists()) {
            openStoredKey()?.let { return it }
            // Declared lost: move everything wrapped aside, then make a new key.
            moveAside()
            return sealNewKey()
        }
        if (databaseExists() || secrets.hasAny()) {
            moveAside()
            return sealNewKey()
        }
        if (quarantine.incidents().isEmpty()) return createFirstKey()
        // Nothing wrapped is left but there is a quarantine: a reset that stopped before the new
        // key was written. Finish it (this is not a first run: the old key may still matter).
        return sealNewKey()
    }

    /** The stored key; null when the loss is declared (definite or confirmed); throws UNAVAILABLE when it is not decided. */
    private fun openStoredKey(): DatabaseKey? {
        val plain = try {
            box.open(keyFile.readBytes(), DB_KEY_CONTEXT)
        } catch (e: SecretBoxException) {
            when (e.kind) {
                SecretBoxException.Kind.CORRUPT -> return null
                SecretBoxException.Kind.KEY_LOST, SecretBoxException.Kind.UNAVAILABLE -> {
                    // Weak evidence has to persist across starts; a definite one is acted on now.
                    if (e.weak && !suspicion.observe()) throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
                    if (e.weak || e.kind == SecretBoxException.Kind.KEY_LOST) return null
                    throw e
                }
                SecretBoxException.Kind.UNSUPPORTED_VERSION -> throw e
            }
        }
        if (plain.size == KEY_BYTES) {
            suspicion.clear()
            return DatabaseKey(plain)
        }
        plain.fill(0)
        return null
    }

    /** The very first key. Nothing wrapped exists, so there is nothing a new Keystore key could destroy. */
    private fun createFirstKey(): DatabaseKey {
        requireNoWrappedFiles()
        keys.create()
        return writeNewKey { box.seal(it, DB_KEY_CONTEXT) }
    }

    /**
     * Seals a new key after a loss. The wrapping key is used if it works; it is replaced only on
     * definite evidence that it is dead, or on weak evidence that has persisted ([suspicion]).
     */
    private fun sealNewKey(): DatabaseKey {
        requireNoWrappedFiles()
        return writeNewKey { key ->
            try {
                box.seal(key, DB_KEY_CONTEXT)
            } catch (e: SecretBoxException) {
                val definite = e.kind == SecretBoxException.Kind.KEY_LOST && !e.weak
                val confirmed = e.weak && suspicion.observe()
                if (!definite && !confirmed) throw if (e.weak) SecretBoxException(SecretBoxException.Kind.UNAVAILABLE) else e
                keys.create()
                box.seal(key, DB_KEY_CONTEXT)
            }
        }
    }

    private fun writeNewKey(seal: (ByteArray) -> ByteArray): DatabaseKey {
        val key = ByteArray(KEY_BYTES)
        random.nextBytes(key)
        val blob = try {
            seal(key)
        } catch (e: SecretBoxException) {
            key.fill(0)
            throw e
        }
        try {
            writeAtomically(keyFile, blob)
        } catch (e: IOException) {
            key.fill(0)
            throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
        }
        suspicion.clear()
        return DatabaseKey(key)
    }

    /** No wrapped file of any kind may be in place when a Keystore key is made: it could make them unreadable. */
    private fun requireNoWrappedFiles() {
        check(!keyFile.exists() && !secrets.hasAny()) { "a wrapping key must not be made while wrapped files exist" }
    }

    private fun databaseExists(): Boolean = databaseSiblings().any { it.exists() }

    private fun databaseSiblings(): List<File> =
        listOf("", "-wal", "-shm", "-journal").map { File(databaseFile.path + it) }

    /**
     * Every file wrapped by the secret box apart from the database key: the account secrets and the app's own (the
     * Companion session). They move aside and come back together; one left behind would block the new key.
     */
    private fun wrappedSecretFiles(): List<File> =
        dir.listFiles { f -> f.name.startsWith(SecretStore.PREFIX) || f.name.startsWith(SecretStore.APP_PREFIX) }?.toList()
            ?: throw IOException("cannot list the secrets")

    private fun moveAside() {
        // The notice first: a crash half way through must still show it.
        val wasPending = keyLoss.pending.value
        keyLoss.raise()
        try {
            incidentThisStart = quarantine.moveAside(
                listOf(
                    "database" to databaseSiblings(),
                    "secrets" to wrappedSecretFiles(),
                    // Last: if the app is killed half way, the wrapped key is still there and the
                    // same decision is made again at the next start.
                    "key" to listOf(keyFile),
                ),
            )
        } catch (e: IOException) {
            // Whatever was not moved is still here, so the next start decides again and raises the
            // notice then. Do not tell the person about a reset that did not happen.
            if (!wasPending) keyLoss.acknowledge()
            throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
        }
    }

    // ---- putting quarantined data back after a false alarm ----

    /**
     * Once per process, before the core is opened: finds the newest incident whose wrapped key
     * opens with the current wrapping key. Restores it at once when nothing new was written (no
     * account secret exists), otherwise offers it. Returns true when files were swapped (the
     * caller then reads the key again). Throws [SecretBoxException] UNAVAILABLE when the Keystore
     * cannot answer now (the check is repeated at the next call).
     */
    private fun checkQuarantine(): Boolean {
        val restored = checkQuarantineOnce()
        restoreChecked = true
        return restored
    }

    private fun checkQuarantineOnce(): Boolean {
        // The incident this very pass made is the app's own, fresh judgment: it is looked at from the next start on.
        val candidate = quarantine.incidents().asReversed().firstOrNull { it != incidentThisStart && openable(it) }
        if (candidate == null) {
            offered = null
            quarantineRestore.set(false)
            return false
        }
        // An incident whose restore was cut short is finished whatever is in the secrets folder now.
        if (!secrets.hasAny() || File(candidate, RESTORING).exists()) {
            performRestore(candidate, supersedeSecrets = false)
            keyLoss.acknowledge()
            offered = null
            quarantineRestore.set(false)
            return true
        }
        offered = candidate
        quarantineRestore.set(true)
        return false
    }

    /** True when the incident has a wrapped key, was not dismissed, and the current wrapping key opens it. */
    private fun openable(incident: File): Boolean {
        val blob = File(incident, "key/${keyFile.name}")
        if (!blob.isFile || File(incident, DECLINED).exists()) return false
        val plain = try {
            box.open(blob.readBytes(), DB_KEY_CONTEXT)
        } catch (e: SecretBoxException) {
            if (e.kind == SecretBoxException.Kind.UNAVAILABLE) throw e
            return false
        }
        val ok = plain.size == KEY_BYTES
        plain.fill(0)
        return ok
    }

    /**
     * Swaps the live files for the incident's. The live database (and the live secrets when
     * [supersedeSecrets]) move into `<incident>/superseded/`, the live key is copied there, then
     * the incident's database, secrets and finally its key move into place; the key goes last and
     * by one atomic rename, and a `restoring` marker makes a run that was cut short start over
     * where it stopped instead of moving the restored files away again.
     */
    private fun performRestore(incident: File, supersedeSecrets: Boolean) {
        try {
            val marker = File(incident, RESTORING)
            if (!marker.exists()) {
                writeAtomically(marker, ByteArray(0))
                val superseded = File(incident, "superseded")
                quarantine.moveInto(File(superseded, "database"), databaseSiblings())
                if (supersedeSecrets) quarantine.moveInto(File(superseded, "secrets"), wrappedSecretFiles())
                if (keyFile.exists()) {
                    File(superseded, "key").mkdirs()
                    keyFile.copyTo(File(superseded, "key/${keyFile.name}"), overwrite = true)
                }
            }
            val home = databaseFile.absoluteFile.parentFile ?: throw IOException("no parent directory")
            quarantine.moveInto(home, File(incident, "database").listFiles()?.toList() ?: emptyList())
            quarantine.moveInto(dir, File(incident, "secrets").listFiles()?.toList() ?: emptyList())
            quarantine.moveInto(dir, File(incident, "key").listFiles()?.toList() ?: emptyList())
        } catch (e: IOException) {
            throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
        }
    }

    /**
     * The first step of "Erase all Croft data on this device": deletes the whole quarantine (every earlier database, wrapped secrets and
     * wrapped key, `superseded/` too), the key-lost notice and the loss-suspected file, and stops offering a restore. **Before the first
     * account is removed**, so that a kill at any later step cannot leave an older database waiting to be restored by a start that finds no
     * account secret (the case in which the app restores at once). Verified: throws [SecureStorageException] (STORAGE_ERROR) when anything
     * of the quarantine is left; the call can be repeated.
     */
    fun eraseQuarantine() {
        assertNotMainThread()
        withLock { wipeQuarantine() }
    }

    private fun wipeQuarantine() {
        if (!quarantine.deleteAll()) throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
        for (f in listOf(File(dir, KEY_LOST_MARKER), File(dir, SUSPICION_FILE))) {
            if (f.exists() && !f.delete()) throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
        }
        keyLoss.acknowledge()
        quarantineRestore.set(false)
        offered = null
        incidentThisStart = null
    }

    /**
     * "Erase all Croft data on this device", the rest: after [eraseQuarantine], deletes the database and its side files (**close the core
     * first**), every account secret, the wrapped database key, and then the wrapping key in the Keystore. Under the process lock. The
     * order is chosen so that a kill at any point leaves nothing a start would restore: the quarantine goes first (also here, again), the
     * database's side files before the database, the database before the wrapped key. Everything is **checked** before the Keystore key
     * is discarded: if anything survived, [SecureStorageException] (STORAGE_ERROR) is thrown and the wrapping key is kept, so that what
     * is still there stays usable (a discarded key with data left would make the next start declare a key loss and quarantine it).
     * Afterwards the next [databaseKey] is a first run. Repeat the call after a failure.
     */
    fun eraseEverything() {
        assertNotMainThread()
        withLock {
            wipeQuarantine()
            val siblings = databaseSiblings()
            for (f in siblings.drop(1) + siblings.first()) {
                if (f.exists() && !f.delete()) throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
            }
            try {
                secrets.clear()
                if (secrets.hasAny()) throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
            } catch (e: IOException) {
                throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
            }
            if (keyFile.exists() && !keyFile.delete()) throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
            // Verified before the Keystore key goes.
            if (quarantine.incidents().isNotEmpty() || databaseExists() || keyFile.exists()) {
                throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
            }
            try { keys.discard() } catch (e: SecretBoxException) { /* replaced at the next first run (`WrappingKeySource.create`) */ }
            restoreChecked = false
            startCounted = false
        }
    }

    internal fun restoreFromQuarantine() {
        assertNotMainThread()
        withLock {
            val incident = offered ?: return@withLock
            performRestore(incident, supersedeSecrets = true)
            keyLoss.acknowledge()
            offered = null
            quarantineRestore.set(false)
        }
    }

    internal fun dismissQuarantineOffer() {
        withLock {
            val incident = offered ?: return@withLock
            try {
                writeAtomically(File(incident, DECLINED), ByteArray(0))
            } catch (e: IOException) {
                throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
            }
            offered = null
            quarantineRestore.set(false)
        }
    }

    private fun notReady() = SecureStorageException(SecureStorageException.Reason.NOT_READY)

    /** The in-process monitor and the file lock: one key decision at a time, across instances and processes. */
    private fun <T> withLock(block: () -> T): T = synchronized(PROCESS_LOCK) {
        dir.mkdirs()
        try {
            FileChannel.open(lockFile.toPath(), StandardOpenOption.CREATE, StandardOpenOption.WRITE).use { channel ->
                channel.lock().use { block() }
            }
        } catch (e: IOException) {
            throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
        }
    }

    override fun toString(): String = "[redacted]"

    companion object {
        private const val KEY_BYTES = 32
        private const val KEY_LOST_MARKER = "key-lost.flag"
        private const val LOCK_FILE = "lock"
        private const val SUSPICION_FILE = "loss-suspected.v1"
        private const val RESTORING = "restoring"
        private const val DECLINED = "declined"
        private val DB_KEY_CONTEXT = "croft/db-key/v1".toByteArray(Charsets.UTF_8)

        /** Waits before each repeat of a key open that failed with UNAVAILABLE. */
        internal val DEFAULT_RETRY_DELAYS_MS = listOf(100L, 500L, 2000L)

        /** One lock for the whole process: a [FileChannel] lock only excludes other processes. */
        private val PROCESS_LOCK = Any()
    }
}

/** Where the wrapped key and the account secrets live, inside the app's no-backup folder. */
internal fun secretsDirectory(context: Context): File = File(context.noBackupFilesDir, "croft-secrets")

/**
 * The app's secrets on a real phone: the wrapping key in the Android Keystore, the files in
 * `noBackupFilesDir` (never part of a backup or a device transfer). One instance per process: a
 * second call for the same folder returns the first one, so two parts of the app cannot race to
 * make the first key. (A different [databaseFile] for the same folder is a bug and throws.)
 */
fun createAppSecrets(context: Context, databaseFile: File): AppSecrets =
    ProcessSecrets.get(secretsDirectory(context), databaseFile) {
        val keys = KeystoreKeySource(context.applicationContext)
        AppSecrets(secretsDirectory(context), AesGcmSecretBox(keys), keys, databaseFile)
    }

/** The one [AppSecrets] of this process for each secrets folder. */
internal object ProcessSecrets {
    private class Entry(val databaseFile: File, val secrets: AppSecrets)

    private val instances = HashMap<String, Entry>()

    @Synchronized
    fun get(dir: File, databaseFile: File, make: () -> AppSecrets): AppSecrets {
        val id = dir.absoluteFile.normalize().path
        val existing = instances[id]
        if (existing != null) {
            check(existing.databaseFile.absoluteFile.normalize() == databaseFile.absoluteFile.normalize()) {
                "AppSecrets already exists for this folder with another database file"
            }
            return existing.secrets
        }
        return make().also { instances[id] = Entry(databaseFile, it) }
    }

    /** For tests that make a new app for each case. */
    @Synchronized
    fun clearForTests() {
        instances.clear()
    }
}
