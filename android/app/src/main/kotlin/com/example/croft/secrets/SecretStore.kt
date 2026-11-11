// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.channels.FileChannel
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption

/** What a stored account secret is. One entry per account and kind. */
enum class SecretKind(internal val wireName: String) {
    PASSWORD("password"),
    OAUTH_REFRESH("oauth_refresh"),
    OAUTH_ACCESS("oauth_access"),
}

/**
 * Account secrets (passwords, OAuth tokens), one wrapped file per `(account id, kind)` in [dir]
 * (the app's `noBackupFilesDir`, never backed up). Every file is the envelope of [box]; the
 * Android Keystore key that opens it never leaves the keystore.
 *
 * Secrets go in and out as `ByteArray`. [put] copies what it is given, so the caller wipes its own
 * array (`fill(0)`) once it returns; the array [get] returns is the caller's to wipe.
 * Nothing here logs, and no exception carries a secret.
 */
class SecretStore(private val dir: File, private val box: SecretBox) {
    init {
        dir.mkdirs()
    }

    /** Stores (or replaces) the secret. Throws [SecretBoxException] when it cannot be sealed. */
    @Synchronized
    fun put(accountId: Long, kind: SecretKind, secret: ByteArray) {
        val blob = box.seal(secret, context(accountId, kind))
        writeAtomically(file(accountId, kind), blob)
    }

    /**
     * Stores the first secret of a new account: first deletes every entry under this id (the core
     * never reuses an id, so there should be none; a leftover of a crash must not be mistaken for
     * the new account's), then stores [secret]. Throws [SecretBoxException] when it cannot be sealed.
     */
    @Synchronized
    fun putForNewAccount(accountId: Long, kind: SecretKind, secret: ByteArray) {
        removeAccount(accountId)
        put(accountId, kind, secret)
    }

    /**
     * The secret, or null when there is none for this account and kind. An entry that cannot be
     * opened because it is damaged or does not belong there is deleted and reads as null, so the
     * person is asked to sign in again. A keystore that fails (KEY_LOST or UNAVAILABLE) and an
     * entry from a newer Croft (UNSUPPORTED_VERSION) are reported with [SecretBoxException] and
     * nothing is deleted.
     */
    @Synchronized
    fun get(accountId: Long, kind: SecretKind): ByteArray? {
        val f = file(accountId, kind)
        val blob = try {
            f.readBytes()
        } catch (e: java.io.FileNotFoundException) {
            return null
        }
        try {
            return box.open(blob, context(accountId, kind))
        } catch (e: SecretBoxException) {
            if (e.kind == SecretBoxException.Kind.CORRUPT) {
                f.delete()
                return null
            }
            throw e
        }
    }

    /** True when an entry exists (whether or not it can be opened). */
    @Synchronized
    fun has(accountId: Long, kind: SecretKind): Boolean = file(accountId, kind).exists()

    /** Deletes one entry. */
    @Synchronized
    fun remove(accountId: Long, kind: SecretKind) {
        file(accountId, kind).delete()
    }

    /**
     * Deletes every entry of an account. Call it **before** `Core.removeAccount` (a crash in
     * between leaves an account that only needs a sign-in; the other order could leave a secret
     * that a later account with the same id would find).
     */
    @Synchronized
    fun removeAccount(accountId: Long) {
        SecretKind.values().forEach { remove(accountId, it) }
    }

    /** True when any account secret exists. */
    @Synchronized
    fun hasAny(): Boolean = dir.listFiles { f -> f.name.startsWith(PREFIX) }?.isNotEmpty() == true

    /** Deletes every account secret (key loss). */
    @Synchronized
    fun clear() {
        dir.listFiles { f -> f.name.startsWith(PREFIX) }?.forEach { it.delete() }
    }

    private fun file(accountId: Long, kind: SecretKind) = File(dir, "$PREFIX$accountId.${kind.wireName}.v1")

    override fun toString(): String = "[redacted]"

    internal companion object {
        const val PREFIX = "acct-"

        /** Binds a blob to its entry, so a file copied onto another account or kind does not open. */
        fun context(accountId: Long, kind: SecretKind): ByteArray =
            "croft/secret/v1/$accountId/${kind.wireName}".toByteArray(Charsets.UTF_8)
    }
}

/**
 * Writes through a temporary file and renames it, so a crash leaves the old file or the new one.
 * The file is synced before the rename and the directory after it, so the rename itself survives
 * a power loss.
 */
internal fun writeAtomically(target: File, bytes: ByteArray) {
    val parent = target.absoluteFile.parentFile ?: error("no parent directory")
    parent.mkdirs()
    val tmp = File(parent, target.name + ".tmp")
    FileOutputStream(tmp).use { out ->
        out.write(bytes)
        out.fd.sync()
    }
    try {
        Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
    } catch (e: AtomicMoveNotSupportedException) {
        Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING)
    }
    syncDirectory(parent)
}

/**
 * Syncs a directory, which is what makes a rename, a creation or a move in it durable. A
 * filesystem that cannot open or sync a directory (an IOException) is tolerated; the sync of the
 * file's own data is not (see [writeAtomically]).
 */
internal fun syncDirectory(dir: File) {
    try {
        FileChannel.open(dir.toPath(), StandardOpenOption.READ).use { it.force(true) }
    } catch (e: IOException) {
        // Not supported here (some filesystems refuse a read-only open of a directory for force).
    }
}
