// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.keys

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.PgpException
import uniffi.croft_ffi.PgpKeyInfo

/** An account as S54 lists it (the sample mailbox is not one: it sends nothing). */
data class KeyAccount(val id: Long, val address: String)

/** What S54 says about a key. */
enum class KeyState {
    /** It signs and encrypts now. */
    ACTIVE,
    EXPIRED,
    REVOKED,

    /** Neither expired nor revoked, but it cannot both sign and encrypt (a key made elsewhere for something else, or too weak). */
    UNUSABLE,
}

/** An account's own key (its secret is in the phone's keystore). `toString` shows the fingerprint only. */
data class OwnKey(
    val fingerprint: String,
    val addresses: List<String>,
    val createdMs: Long,
    val expiresMs: Long?,
    val state: KeyState,
    /** Imported from a file (else made on this phone). */
    val imported: Boolean,
) {
    /** The fingerprint in groups of four, for reading aloud and comparing. */
    val groupedFingerprint: String get() = fingerprint.chunked(4).joinToString(" ")

    /** The last eight characters, as two groups ("Key 3F2A 91C4" on S54). */
    val shortFingerprint: String get() = fingerprint.takeLast(8).chunked(4).joinToString(" ")

    override fun toString(): String = "OwnKey($fingerprint, $state)"
}

/** Why a key call failed: a fixed category, never key material or a passphrase. */
enum class KeysProblem { NO_VAULT, VAULT_FAILED, UNREADABLE, NOT_ONE_KEY, TOO_LARGE, WRONG_PASSPHRASE, NO_USABLE_KEY, GONE, FAILED }

/** A key call that was refused. */
class KeysFailure(val problem: KeysProblem) : Exception(problem.name)

/** The core's key calls that S54 and S55 use (FFI v23). Every call is `suspend` and runs off the main thread. */
interface KeysBackend {
    /** The accounts that can have a key, in the core's order. */
    suspend fun accounts(): List<KeyAccount>

    /** The account's own key in use (the newest that signs and encrypts, else the newest), or null. */
    suspend fun ownKey(account: Long): OwnKey?

    /** Makes a key for the account on this phone (its address as the user ID). */
    suspend fun generate(account: Long): OwnKey

    /**
     * Imports a key file or pasted text. A secret key becomes the account's own (its secret into the keystore); [passphrase] opens
     * a protected one (empty for none). Returns the account's own key afterwards, or null when the file was someone else's public
     * key (it is kept for writing to them).
     */
    suspend fun import(account: Long, bytes: ByteArray, passphrase: String): OwnKey?

    /** The public key, armored, to share. */
    suspend fun exportPublic(account: Long, fingerprint: String): String

    /** A backup of the secret key, armored and protected with [passphrase] (not empty). */
    suspend fun exportSecret(account: Long, fingerprint: String, passphrase: String): String

    /** Removes the key from the phone (and its secret from the keystore). False when it was already gone. */
    suspend fun remove(account: Long, fingerprint: String): Boolean
}

internal fun PgpKeyInfo.toOwnKey() = OwnKey(
    fingerprint = fingerprint,
    addresses = addresses,
    createdMs = createdMs,
    expiresMs = expiresMs,
    state = when {
        revoked -> KeyState.REVOKED
        expired -> KeyState.EXPIRED
        canEncrypt && canSign -> KeyState.ACTIVE
        else -> KeyState.UNUSABLE
    },
    imported = source == uniffi.croft_ffi.PgpKeySource.IMPORTED,
)

internal fun PgpException.problem(): KeysProblem = when (this) {
    is PgpException.NoVault -> KeysProblem.NO_VAULT
    is PgpException.VaultFailed -> KeysProblem.VAULT_FAILED
    is PgpException.Unreadable -> KeysProblem.UNREADABLE
    is PgpException.NotOneKey -> KeysProblem.NOT_ONE_KEY
    is PgpException.TooLarge -> KeysProblem.TOO_LARGE
    is PgpException.WrongPassphrase -> KeysProblem.WRONG_PASSPHRASE
    is PgpException.NoUsableKey -> KeysProblem.NO_USABLE_KEY
    is PgpException.NoSuchKey, is PgpException.NoSuchAccount -> KeysProblem.GONE
    else -> KeysProblem.FAILED
}

/** The key in use among [keys] (own keys only): the newest that signs and encrypts, else the newest. */
internal fun inUse(keys: List<OwnKey>): OwnKey? =
    keys.sortedByDescending { it.createdMs }.let { sorted -> sorted.firstOrNull { it.state == KeyState.ACTIVE } ?: sorted.firstOrNull() }

/** [KeysBackend] over the core. */
class CoreKeysBackend(private val core: Core, private val io: CoroutineDispatcher = Dispatchers.IO) : KeysBackend {
    private suspend fun <T> call(block: () -> T): T = withContext(io) {
        try {
            block()
        } catch (e: PgpException) {
            throw KeysFailure(e.problem())
        }
    }

    override suspend fun accounts(): List<KeyAccount> = withContext(io) {
        try {
            core.listAccounts().filter { !it.isDemo }.map { KeyAccount(it.id, it.email) }
        } catch (e: uniffi.croft_ffi.CoreException) {
            emptyList()
        }
    }

    override suspend fun ownKey(account: Long): OwnKey? = call { inUse(core.pgpKeys(account).filter { it.own }.map { it.toOwnKey() }) }

    override suspend fun generate(account: Long): OwnKey = call { core.generateKey(account).toOwnKey() }

    override suspend fun import(account: Long, bytes: ByteArray, passphrase: String): OwnKey? = call {
        try {
            val key = core.importKey(account, bytes, passphrase)
            if (key.own) key.toOwnKey() else null
        } finally {
            bytes.fill(0)
        }
    }

    override suspend fun exportPublic(account: Long, fingerprint: String): String = call { core.exportPublicKey(account, fingerprint) }

    override suspend fun exportSecret(account: Long, fingerprint: String, passphrase: String): String =
        call { core.exportSecretKey(account, fingerprint, passphrase) }

    override suspend fun remove(account: Long, fingerprint: String): Boolean = call { core.deleteKey(account, fingerprint) }
}

/** A [KeysBackend] in memory, for tests, previews and screenshots. Keys are made-up records; nothing here is real key material. */
class FakeKeysBackend(
    accounts: List<KeyAccount> = listOf(KeyAccount(1, "sam@example.org"), KeyAccount(2, "sam@acme.test")),
    private val clock: () -> Long = { 1_793_750_400_000L },
) : KeysBackend {
    private val accountList = accounts.toList()
    val keys = mutableMapOf<Long, MutableList<OwnKey>>()

    /** The passphrase a "protected" imported file needs (any bytes starting with `PROTECTED`). */
    var importPassphrase = "made-up passphrase"

    /** When set, every call fails with it. */
    var failWith: KeysProblem? = null

    /** What [exportSecret] was asked to protect the backup with, and what was imported (as text), for tests. */
    val backups = mutableListOf<String>()
    val imported = mutableListOf<String>()

    private var made = 0

    private fun check() {
        failWith?.let { throw KeysFailure(it) }
    }

    private fun fresh(account: Long, imported: Boolean): OwnKey {
        made++
        val fp = "%040X".format(java.math.BigInteger.valueOf(0x3F2A91C4L * 1000 + account * 100 + made))
        val address = accountList.first { it.id == account }.address
        return OwnKey(fp, listOf(address), clock(), clock() + 2L * 365 * 24 * 3600 * 1000, KeyState.ACTIVE, imported)
    }

    override suspend fun accounts(): List<KeyAccount> = accountList.also { check() }

    override suspend fun ownKey(account: Long): OwnKey? {
        check()
        return inUse(keys[account].orEmpty())
    }

    override suspend fun generate(account: Long): OwnKey {
        check()
        return fresh(account, imported = false).also { keys.getOrPut(account) { mutableListOf() } += it }
    }

    override suspend fun import(account: Long, bytes: ByteArray, passphrase: String): OwnKey? {
        check()
        val text = String(bytes, Charsets.UTF_8)
        bytes.fill(0)
        imported += text
        if (text.isBlank() || !text.contains("PGP")) throw KeysFailure(KeysProblem.UNREADABLE)
        if (text.startsWith("PROTECTED") && passphrase != importPassphrase) throw KeysFailure(KeysProblem.WRONG_PASSPHRASE)
        if (!text.contains("PRIVATE")) return null
        return fresh(account, imported = true).also { keys.getOrPut(account) { mutableListOf() } += it }
    }

    override suspend fun exportPublic(account: Long, fingerprint: String): String {
        check()
        return "-----BEGIN PGP PUBLIC KEY BLOCK-----\n\n(made up for $fingerprint)\n-----END PGP PUBLIC KEY BLOCK-----\n"
    }

    override suspend fun exportSecret(account: Long, fingerprint: String, passphrase: String): String {
        check()
        if (passphrase.isEmpty()) throw KeysFailure(KeysProblem.WRONG_PASSPHRASE)
        backups += passphrase
        return "-----BEGIN PGP PRIVATE KEY BLOCK-----\n\n(made up)\n-----END PGP PRIVATE KEY BLOCK-----\n"
    }

    override suspend fun remove(account: Long, fingerprint: String): Boolean {
        check()
        return keys[account]?.removeIf { it.fingerprint == fingerprint } ?: false
    }
}
