// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.content.Context
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

/**
 * The app's secrets: the database key, the account secrets and the key-loss notice, all kept in
 * [dir] (the app's `noBackupFilesDir/croft-secrets`) and wrapped by [box]. Use one instance per
 * process (`createAppSecrets` does); [databaseKey] also takes a file lock, so a second process or
 * a second instance cannot make a second key while the first is being made.
 *
 * [databaseFile] is the SQLite file the key belongs to. It is only touched when the key is
 * definitely lost: then it (and its `-wal`, `-shm` and `-journal` files), every account secret and
 * the wrapped key are **moved**, not deleted, into `croft-quarantine/<timestamp>/` next to [dir]
 * (see [Quarantine]), a new key is made and [keyLoss] is raised. The mail is synced again from
 * the servers; the person signs in again. Quarantined data is deleted after 7 days and 3 later
 * successful starts. The Keystore entry is kept, so that quarantined data can still be opened if
 * the loss was a wrong call; it is replaced only when the entry is itself the dead key.
 *
 * What counts as lost: the wrapped key's envelope is damaged or fails authentication
 * ([SecretBoxException.Kind.CORRUPT]), the Keystore says the key is definitely gone
 * (KEY_LOST, see [KeystoreErrors]), or data exists without its wrapped key. A Keystore that
 * merely fails (UNAVAILABLE) is retried with backoff and then reported as
 * [SecureStorageException.Reason.NOT_READY]; a blob from a newer Croft (UNSUPPORTED_VERSION) as
 * NEWER_DATA. Neither moves or deletes anything.
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
    )

    init {
        dir.mkdirs()
    }

    /** Account passwords and tokens. */
    val secrets = SecretStore(dir, box)

    /** Raised when the keystore key was gone and everything was moved aside. */
    val keyLoss = KeyLossNotice(File(dir, KEY_LOST_MARKER))

    private val keyFile = File(dir, "db-key.v1")
    private val lockFile = File(dir, LOCK_FILE)
    private var startCounted = false

    /**
     * The database key: made on first run, opened from its wrapped file afterwards. A key that is
     * definitely lost is handled as described in the class text. Throws [SecureStorageException]
     * when the storage cannot be used now (nothing is moved or deleted then). The first success in
     * a process also counts as a successful start for the quarantine's housekeeping.
     * Wipe or [DatabaseKey.close] the result.
     */
    fun databaseKey(): DatabaseKey = withLock {
        var incident: File? = null
        val key = openStoredKey() ?: run {
            // Lost: the old data is moved aside, then a new key is made.
            incident = moveAside()
            create()
        }
        if (!startCounted) {
            startCounted = true
            quarantine.recordSuccessfulStart(except = incident)
        }
        key
    }

    /** The stored key; null when it is definitely lost or when data exists without one. */
    private fun openStoredKey(): DatabaseKey? {
        if (!keyFile.exists()) {
            return if (databaseExists() || secrets.hasAny()) null else create()
        }
        val plain = try {
            retrying { box.open(keyFile.readBytes(), DB_KEY_CONTEXT) }
        } catch (e: SecretBoxException) {
            when (e.kind) {
                SecretBoxException.Kind.KEY_LOST, SecretBoxException.Kind.CORRUPT -> return null
                SecretBoxException.Kind.UNSUPPORTED_VERSION ->
                    throw SecureStorageException(SecureStorageException.Reason.NEWER_DATA)
                SecretBoxException.Kind.UNAVAILABLE -> throw notReady()
            }
        }
        if (plain.size == KEY_BYTES) return DatabaseKey(plain)
        plain.fill(0)
        return null
    }

    private fun create(): DatabaseKey {
        val key = ByteArray(KEY_BYTES)
        random.nextBytes(key)
        val blob = try {
            retrying { sealFirstKey(key) }
        } catch (e: SecretBoxException) {
            key.fill(0)
            throw notReady()
        }
        try {
            writeAtomically(keyFile, blob)
        } catch (e: IOException) {
            key.fill(0)
            throw SecureStorageException(SecureStorageException.Reason.STORAGE_ERROR)
        }
        return DatabaseKey(key)
    }

    /**
     * Seals the new key. If the wrapping key itself definitely refuses (the entry is the dead
     * key), the entry is deleted once and a fresh one is made: that is the one place the alias is
     * not kept.
     */
    private fun sealFirstKey(key: ByteArray): ByteArray =
        try {
            box.seal(key, DB_KEY_CONTEXT)
        } catch (e: SecretBoxException) {
            if (e.kind != SecretBoxException.Kind.KEY_LOST) throw e
            keys.discard()
            box.seal(key, DB_KEY_CONTEXT)
        }

    private fun databaseExists(): Boolean = databaseSiblings().any { it.exists() }

    private fun databaseSiblings(): List<File> =
        listOf("", "-wal", "-shm", "-journal").map { File(databaseFile.path + it) }

    private fun moveAside(): File {
        // The notice first: a crash half way through must still show it.
        val wasPending = keyLoss.pending.value
        keyLoss.raise()
        try {
            return quarantine.moveAside(
                listOf(
                    "database" to databaseSiblings(),
                    "secrets" to (dir.listFiles { f -> f.name.startsWith(SecretStore.PREFIX) }?.toList() ?: emptyList()),
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

    /** Runs [op], repeating it after each of [retryDelaysMs] while it fails with UNAVAILABLE. */
    private fun <T> retrying(op: () -> T): T {
        var attempt = 0
        while (true) {
            try {
                return op()
            } catch (e: SecretBoxException) {
                if (e.kind != SecretBoxException.Kind.UNAVAILABLE || attempt >= retryDelaysMs.size) throw e
                sleep(retryDelaysMs[attempt++])
            }
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
