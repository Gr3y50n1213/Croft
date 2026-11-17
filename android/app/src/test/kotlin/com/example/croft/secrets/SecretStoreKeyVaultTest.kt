// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.PgpException
import uniffi.croft_ffi.ProviderException
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/**
 * T42a: the core's OpenPGP secret keys in the Keystore-backed store (over an in-memory wrapping key), against the real Rust core
 * (the host library through JNA). Made, read back, exported, removed; never in the database; a Keystore that cannot answer
 * makes no key.
 */
class SecretStoreKeyVaultTest {
    private lateinit var dir: File
    private val keys = MemoryKeySource()
    private lateinit var store: SecretStore
    private val signIns = SignInNeeded()
    private val open = mutableListOf<Core>()

    @Before
    fun setUp() {
        dir = createTempDirectory("croft-vault-test").toFile()
        store = SecretStore(File(dir, "secrets"), AesGcmSecretBox(keys))
    }

    @After
    fun tearDown() {
        open.forEach { it.close() }
        dir.deleteRecursively()
    }

    private fun core(): Core =
        Core.open(File(dir, "croft.db").path, null, SecretStoreCredentials(store, signIns)).also {
            it.setKeyVault(SecretStoreKeyVault(store))
            open += it
        }

    private fun account(core: Core): Long = runBlocking {
        AccountSecrets(store, signIns).addPasswordAccount(
            core,
            FfiDiscovered(
                email = "alice@example.org",
                imap = ServerSettings("127.0.0.1", 1u, SecurityMode.TLS, "alice"),
                smtp = null,
                jmap = null,
                source = DiscoverySource.AUTOCONFIG,
                provider = null,
                authHint = AuthKind.PASSWORD,
            ),
            "Alice",
            null,
            null,
            Fixture.PASSWORD.toByteArray(),
        )
    }

    /** The binary of an armored OpenPGP block. */
    private fun dearmor(armored: String): ByteArray {
        val lines = armored.lines().map { it.trim() }
        val start = lines.indexOfFirst { it.isEmpty() } + 1
        val body = lines.drop(start).takeWhile { !it.startsWith("=") && !it.startsWith("-----END") }.joinToString("")
        return java.util.Base64.getDecoder().decode(body)
    }

    /** The 24-byte runs of [secret] that do not occur in [public]. */
    private fun secretOnlyRuns(secret: ByteArray, public: ByteArray): List<ByteArray> =
        (0..secret.size - RUN).map { secret.copyOfRange(it, it + RUN) }.filter { !containsBytes(public, it) }

    private val RUN = 24

    private fun databaseBytes(): ByteArray =
        dir.listFiles { f -> f.name.startsWith("croft.db") }!!.fold(ByteArray(0)) { acc, f -> acc + f.readBytes() }

    @Test
    fun aKeyMadeByTheCoreIsSealedInTheStoreAndNeverInTheDatabase() {
        val core = core()
        val id = account(core)
        val made = core.generateKey(id)
        assertTrue(made.own)
        val secret = store.getKey(id, made.fingerprint)
        assertNotNull("the secret is in the vault", secret)
        val file = File(dir, "secrets/acct-$id.pgp-${made.fingerprint}.v1")
        assertTrue(file.exists())
        // Sealed: no run of the secret's bytes that is not also in the public certificate (which the database keeps, signatures
        // and all) is in the store's file or anywhere in the database's files.
        val public = dearmor(core.exportPublicKey(id, made.fingerprint))
        val secretOnly = secretOnlyRuns(secret!!, public)
        assertTrue("the secret has material of its own", secretOnly.isNotEmpty())
        val sealed = file.readBytes()
        val database = databaseBytes()
        assertTrue(secretOnly.none { containsBytes(sealed, it) })
        assertTrue(secretOnly.none { containsBytes(database, it) })
        secret.fill(0)
        // The core reads it back through the vault: a backup opens only with its passphrase.
        assertTrue(core.exportSecretKey(id, made.fingerprint, "made-up passphrase").startsWith("-----BEGIN PGP PRIVATE KEY BLOCK-----"))
        // Removed by the core: gone from the vault too.
        assertTrue(core.deleteKey(id, made.fingerprint))
        assertNull(store.getKey(id, made.fingerprint))
        assertFalse(file.exists())
    }

    @Test
    fun aKeystoreThatCannotAnswerMakesNoKeyAndIsNotReadAsALoss() {
        val core = core()
        val id = account(core)
        val made = core.generateKey(id)
        keys.unavailable = true
        try {
            core.generateKey(id)
            fail("expected the vault's failure")
        } catch (e: PgpException) {
            // VaultFailed: nothing was made
        }
        assertEquals(1, core.pgpKeys(id).size)
        // Reading while the Keystore cannot answer: "not ready", never "no key" (nothing is deleted).
        try {
            SecretStoreKeyVault(store).secretKey(id, made.fingerprint)
            fail("expected ProviderException.NotReady")
        } catch (e: ProviderException.NotReady) {
            // the core uses no key now
        }
        keys.unavailable = false
        assertNotNull(store.getKey(id, made.fingerprint))
    }

    @Test
    fun theArrayTheCoreHandsOverIsWipedAndAFingerprintThatIsNotOneIsRefused() {
        val vault = SecretStoreKeyVault(store)
        val handed = "made-up secret key bytes".toByteArray()
        vault.storeSecretKey(7, "3F2A91C47B10E5D28A6C40B19F7E62C3D8150A4B", handed)
        assertTrue(handed.all { it == 0.toByte() })
        try {
            vault.storeSecretKey(7, "../escape", "x".toByteArray())
            fail("expected ProviderException.Failed")
        } catch (e: ProviderException.Failed) {
            // refused
        }
        assertFalse(SecretStoreKeyVault(store).toString().contains("made-up"))
    }

    @Test
    fun aDamagedEntryIsAFailureAndIsKept() {
        val fp = "3F2A91C47B10E5D28A6C40B19F7E62C3D8150A4B"
        val vault = SecretStoreKeyVault(store)
        vault.storeSecretKey(7, fp, "made-up secret key bytes".toByteArray())
        val file = File(dir, "secrets/acct-7.pgp-$fp.v1")
        val bytes = file.readBytes()
        bytes[bytes.size - 1] = (bytes[bytes.size - 1].toInt() xor 1).toByte()
        file.writeBytes(bytes)
        try {
            vault.secretKey(7, fp)
            fail("expected ProviderException.Failed")
        } catch (e: ProviderException.Failed) {
            // not "no key": the core uses no key now, and nothing is deleted
        }
        assertTrue("a private key's entry is never deleted for not opening", file.exists())
    }
}

