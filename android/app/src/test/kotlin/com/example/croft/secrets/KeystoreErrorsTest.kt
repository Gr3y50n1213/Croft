// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.security.keystore.KeyPermanentlyInvalidatedException
import java.security.InvalidKeyException
import java.security.KeyStoreException
import java.security.NoSuchAlgorithmException
import java.security.ProviderException
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import javax.crypto.BadPaddingException
import javax.crypto.IllegalBlockSizeException
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

/**
 * What each kind of Keystore failure means. Fakes throw each exception type; only definite
 * evidence is KEY_LOST, only an authentication failure is CORRUPT, everything else is UNAVAILABLE.
 */
class KeystoreErrorsTest {
    /** Stands in for `android.security.KeyStoreException` (API 33), whose fields a fake cannot set. */
    private class FakePlatformError(val transient: Boolean, val code: Int) : Exception("platform")

    private fun lookup(t: Throwable): PlatformKeystoreError? =
        (t as? FakePlatformError)?.let { PlatformKeystoreError(it.transient, it.code) }

    private fun errors(sdk: Int = 34) = KeystoreErrors(sdk, ::lookup)

    private fun kind(failure: Throwable, sdk: Int = 34) = errors(sdk).classify(failure)

    @Test
    fun aPermanentlyInvalidatedKeyIsKeyLostAnywhereInTheCauseChain() {
        assertEquals(SecretBoxException.Kind.KEY_LOST, kind(KeyPermanentlyInvalidatedException()))
        val wrapped = IllegalStateException("outer", RuntimeException("middle", KeyPermanentlyInvalidatedException()))
        assertEquals(SecretBoxException.Kind.KEY_LOST, kind(wrapped))
        // Whatever the API level.
        assertEquals(SecretBoxException.Kind.KEY_LOST, kind(KeyPermanentlyInvalidatedException(), sdk = 26))
    }

    @Test
    fun aNonTransientKeySpecificPlatformErrorIsKeyLostFromApi33() {
        for (code in listOf(KeystoreErrors.ERROR_KEY_DOES_NOT_EXIST, KeystoreErrors.ERROR_KEY_CORRUPTED)) {
            val failure = UnrecoverableKeyException("Failed to obtain key").initCause(FakePlatformError(false, code))
            assertEquals("code $code", SecretBoxException.Kind.KEY_LOST, kind(failure))
        }
    }

    @Test
    fun aTransientOrOtherPlatformErrorIsUnavailable() {
        // Transient, even with a key-specific code.
        assertEquals(
            SecretBoxException.Kind.UNAVAILABLE,
            kind(UnrecoverableKeyException().initCause(FakePlatformError(true, KeystoreErrors.ERROR_KEY_CORRUPTED))),
        )
        // Not transient, but the code says nothing about the key (system error, permission, uninitialised,
        // user authentication, keymint failure, expired operation...).
        for (code in listOf(1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16)) {
            assertEquals(
                "code $code",
                SecretBoxException.Kind.UNAVAILABLE,
                kind(UnrecoverableKeyException().initCause(FakePlatformError(false, code))),
            )
        }
    }

    @Test
    fun thePlatformErrorCountsOnlyFromApi33() {
        val failure = UnrecoverableKeyException().initCause(FakePlatformError(false, KeystoreErrors.ERROR_KEY_DOES_NOT_EXIST))
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kind(failure, sdk = 32))
        assertEquals(SecretBoxException.Kind.KEY_LOST, kind(failure, sdk = 33))
    }

    @Test
    fun onlyAnAuthenticationFailureIsCorrupt() {
        assertEquals(SecretBoxException.Kind.CORRUPT, kind(AEADBadTagException()))
        // BadPaddingException is the parent class: a keystore failure can arrive as one.
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kind(BadPaddingException()))
    }

    @Test
    fun theFailuresThePlatformUsesForTransientProblemsAreUnavailableNeverKeyLost() {
        val transient = listOf(
            UnrecoverableKeyException("Key blob corrupted? maybe not"),
            InvalidKeyException("Keystore operation failed"),
            IllegalBlockSizeException("blocked"),
            BadPaddingException("padding"),
            ProviderException("Keystore not reachable"),
            KeyStoreException("Keystore not loaded"),
            NoSuchAlgorithmException(),
            RuntimeException("boom"),
            IllegalStateException("not initialised"),
            Exception(),
        )
        transient.forEach { assertEquals(it.javaClass.simpleName, SecretBoxException.Kind.UNAVAILABLE, kind(it)) }
    }

    @Test
    fun aCycleInTheCauseChainTerminates() {
        val a = RuntimeException("a")
        val b = RuntimeException("b", a)
        a.initCause(b)
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kind(a))
        assertEquals(2, KeystoreErrors.causeChain(a).size)
    }

    // ---- end to end through the real JCE cipher: the key throws when the cipher reads it ----

    private fun boxFailing(failure: Throwable) = AesGcmSecretBox(FixedKeySource(FailingKey(failure)))

    private fun sealKind(box: SecretBox): SecretBoxException.Kind {
        try {
            box.seal("x".toByteArray(), "ctx".toByteArray())
        } catch (e: SecretBoxException) {
            return e.kind
        }
        fail("expected a SecretBoxException")
        error("unreachable")
    }

    private fun openKind(box: SecretBox): SecretBoxException.Kind {
        val good = AesGcmSecretBox(MemoryKeySource()).seal("x".toByteArray(), "ctx".toByteArray())
        try {
            box.open(good, "ctx".toByteArray())
        } catch (e: SecretBoxException) {
            return e.kind
        }
        fail("expected a SecretBoxException")
        error("unreachable")
    }

    @Test
    fun theBoxMapsEachThrownTypeOnSealAndOnOpen() {
        val cases = mapOf(
            KeyPermanentlyInvalidatedException() to SecretBoxException.Kind.KEY_LOST,
            InvalidKeyException("plain") to SecretBoxException.Kind.UNAVAILABLE,
            UnrecoverableKeyException("unrecoverable") to SecretBoxException.Kind.UNAVAILABLE,
            ProviderException("provider") to SecretBoxException.Kind.UNAVAILABLE,
            IllegalStateException("state") to SecretBoxException.Kind.UNAVAILABLE,
        )
        cases.forEach { (failure, expected) ->
            val name = failure.javaClass.simpleName
            assertEquals("seal $name", expected, sealKind(boxFailing(failure)))
            assertEquals("open $name", expected, openKind(boxFailing(failure)))
        }
    }

    @Test
    fun aBlobMadeUnderAnotherKeyIsCorruptBecauseTheTagFails() {
        // `openKind` seals with one key and opens with the box it is given: a different one here.
        val other = AesGcmSecretBox(MemoryKeySource().also { it.keyForSealing() })
        assertEquals(SecretBoxException.Kind.CORRUPT, openKind(other))
    }
}
