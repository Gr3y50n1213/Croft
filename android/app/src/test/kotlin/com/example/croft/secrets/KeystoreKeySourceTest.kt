// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.security.keystore.KeyPermanentlyInvalidatedException
import java.security.ProviderException
import java.security.UnrecoverableKeyException
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * The decisions around the Keystore: when a missing key counts as gone, and that nothing is
 * generated or called lost on a failure that may pass. The platform calls are a fake.
 */
class KeystoreKeySourceTest {
    private val ks = FakeKeystore()
    private val sleeps = mutableListOf<Long>()
    private val key: SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    private fun source() =
        KeystoreKeySource("croft.test", ks, KeystoreErrors(sdkInt = 34) { null }, listOf(100L, 500L)) { sleeps += it }

    private fun kindOf(block: () -> Unit): SecretBoxException.Kind {
        try {
            block()
        } catch (e: SecretBoxException) {
            return e.kind
        }
        fail("expected a SecretBoxException")
        error("unreachable")
    }

    @Test
    fun aKeyThatIsThereIsReturnedAtOnceWithNoWaiting() {
        ks.getKeyDefault = Result.success(key)
        assertEquals(key, source().keyForOpening())
        assertEquals(key, source().keyForSealing())
        assertTrue(sleeps.isEmpty())
        assertTrue(ks.generated.isEmpty())
    }

    @Test
    fun aMissingKeyIsLostOnlyAfterThreeChecksSpacedWithBackoffAndOtherCallsWorking() {
        // getKey returns null and aliases() works and lacks the alias, every time.
        ks.aliasesDefault = Result.success(listOf("some.other.alias"))
        assertEquals(SecretBoxException.Kind.KEY_LOST, kindOf { source().keyForOpening() })
        assertEquals(3, ks.getKeyCalls.get())
        assertEquals(3, ks.aliasCalls.get())
        assertEquals(listOf(100L, 500L), sleeps)
        assertTrue(ks.generated.isEmpty() && ks.deleted.isEmpty())
    }

    @Test
    fun aNullKeyThatIsStillListedIsATransientFailureNotALoss() {
        // Android 8 to 11: getKey returns null when the service is unreachable; the alias is listed.
        ks.aliasesDefault = Result.success(listOf("croft.test"))
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForOpening() })
        assertEquals(3, ks.getKeyCalls.get())
    }

    @Test
    fun aNullKeyThatComesBackOnALaterTryIsUsed() {
        ks.getKeySteps += Result.success(null)
        ks.getKeySteps += Result.success(key)
        ks.aliasSteps += Result.success(emptyList())
        assertEquals(key, source().keyForOpening())
        assertEquals(listOf(100L), sleeps)
    }

    @Test
    fun aMixedAnswerIsNotEvidence() {
        // absent, failing, absent: not three clean checks in a row.
        ks.aliasesDefault = Result.success(emptyList())
        ks.getKeySteps += Result.success(null)
        ks.getKeySteps += Result.failure(ProviderException("service down"))
        ks.getKeySteps += Result.success(null)
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForOpening() })
    }

    @Test
    fun whenAliasesItselfFailsTheKeyIsNotCalledLost() {
        ks.aliasesDefault = Result.failure(ProviderException("service down"))
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForOpening() })
    }

    @Test
    fun anUnrecoverableKeyExceptionAloneIsNotALoss() {
        ks.getKeyDefault = Result.failure(UnrecoverableKeyException("Failed to obtain key"))
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForOpening() })
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForSealing() })
        assertTrue("nothing generated over a key that may still be there", ks.generated.isEmpty())
    }

    @Test
    fun aPermanentlyInvalidatedKeyIsLostAtOnce() {
        ks.getKeyDefault = Result.failure(UnrecoverableKeyException().initCause(KeyPermanentlyInvalidatedException()))
        assertEquals(SecretBoxException.Kind.KEY_LOST, kindOf { source().keyForOpening() })
        assertEquals(1, ks.getKeyCalls.get())
        assertTrue(sleeps.isEmpty())
    }

    @Test
    fun sealingGeneratesOnlyAKeyThatIsConfirmedAbsent() {
        ks.aliasesDefault = Result.success(emptyList())
        assertNotNull(source().keyForSealing())
        assertEquals(listOf("croft.test"), ks.generated)
        assertEquals(3, ks.getKeyCalls.get())
    }

    @Test
    fun sealingNeverGeneratesWhileTheKeystoreIsUnsure() {
        ks.aliasesDefault = Result.success(listOf("croft.test"))
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForSealing() })
        ks.getKeyDefault = Result.failure(ProviderException("down"))
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForSealing() })
        assertTrue(ks.generated.isEmpty())
    }

    @Test
    fun aDeadKeyIsReplacedWhenSealing() {
        ks.getKeyDefault = Result.failure(UnrecoverableKeyException().initCause(KeyPermanentlyInvalidatedException()))
        assertNotNull(source().keyForSealing())
        assertEquals(listOf("croft.test"), ks.generated)
    }

    @Test
    fun aFailedGenerationIsUnavailable() {
        ks.aliasesDefault = Result.success(emptyList())
        ks.generateFails = ProviderException("no secure hardware")
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForSealing() })
    }
}
