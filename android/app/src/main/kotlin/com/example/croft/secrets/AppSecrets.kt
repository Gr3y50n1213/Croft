// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.content.Context
import java.io.File
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
 * Tells the first screen that the database and the account secrets were deleted because the
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
 * [dir] (the app's `noBackupFilesDir`) and wrapped by [box].
 *
 * [databaseFile] is the SQLite file the key belongs to. It is only touched when the key is lost:
 * then it (and its `-wal`, `-shm` and `-journal` files), every account secret and the wrapped key
 * are deleted, a new key is made and [keyLoss] is raised. The mail is synced again from the
 * servers; the person signs in again.
 */
class AppSecrets(
    private val dir: File,
    private val box: SecretBox,
    private val keys: WrappingKeySource,
    private val databaseFile: File,
    private val random: SecureRandom = SecureRandom(),
) {
    init {
        dir.mkdirs()
    }

    /** Account passwords and tokens. */
    val secrets = SecretStore(dir, box)

    /** Raised when the keystore key was gone and everything was reset. */
    val keyLoss = KeyLossNotice(File(dir, KEY_LOST_MARKER))

    private val keyFile = File(dir, "db-key.v1")

    /**
     * The database key: made on first run, opened from its wrapped file afterwards. A key that is
     * definitely lost (the wrapped file cannot be opened, or the file is missing while data exists)
     * resets everything as described in the class text. A keystore that merely fails now
     * ([SecretBoxException] UNAVAILABLE) throws and deletes nothing: try again later.
     * Wipe or [DatabaseKey.close] the result.
     */
    @Synchronized
    fun databaseKey(): DatabaseKey {
        if (keyFile.exists()) {
            try {
                val plain = box.open(keyFile.readBytes(), DB_KEY_CONTEXT)
                if (plain.size == KEY_BYTES) return DatabaseKey(plain)
                plain.fill(0)
            } catch (e: SecretBoxException) {
                if (e.kind == SecretBoxException.Kind.UNAVAILABLE) throw e
            }
            reset()
        } else if (databaseExists() || secrets.hasAny()) {
            // Data without its key: it can never be read.
            reset()
        }
        return create()
    }

    private fun create(): DatabaseKey {
        val key = ByteArray(KEY_BYTES)
        random.nextBytes(key)
        val blob = box.seal(key, DB_KEY_CONTEXT)
        writeAtomically(keyFile, blob)
        return DatabaseKey(key)
    }

    private fun databaseExists(): Boolean = databaseSiblings().any { it.exists() }

    private fun databaseSiblings(): List<File> =
        listOf("", "-wal", "-shm", "-journal").map { File(databaseFile.path + it) }

    private fun reset() {
        // The notice first: a crash half way through the reset must still show it.
        keyLoss.raise()
        databaseSiblings().forEach { it.delete() }
        secrets.clear()
        keyFile.delete()
        keys.discard()
    }

    override fun toString(): String = "[redacted]"

    companion object {
        private const val KEY_BYTES = 32
        private const val KEY_LOST_MARKER = "key-lost.flag"
        private val DB_KEY_CONTEXT = "croft/db-key/v1".toByteArray(Charsets.UTF_8)
    }
}

/** Where the wrapped key and the account secrets live, inside the app's no-backup folder. */
internal fun secretsDirectory(context: Context): File = File(context.noBackupFilesDir, "croft-secrets")

/**
 * The app's secrets on a real phone: the wrapping key in the Android Keystore, the files in
 * `noBackupFilesDir` (never part of a backup or a device transfer).
 */
fun createAppSecrets(context: Context, databaseFile: File): AppSecrets {
    val keys = KeystoreKeySource(context.applicationContext)
    return AppSecrets(secretsDirectory(context), AesGcmSecretBox(keys), keys, databaseFile)
}
