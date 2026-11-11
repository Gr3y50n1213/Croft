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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * The decisions around the Keystore: when a missing key counts as gone, and that nothing is
 * generated or called lost on a failure that may pass. The platform calls are a fake.
 *
 * The alias is `croft.test` and its canary `croft.test.canary`. "Lost" needs the canary listed and
 * usable while the main alias is absent, on all three checks.
 */
class KeystoreKeySourceTest {
    private val ks = FakeKeystore()
    private val sleeps = mutableListOf<Long>()
    private val key: SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    private fun source(sdkInt: Int = 34) =
        KeystoreKeySource("croft.test", ks, KeystoreErrors(sdkInt = sdkInt) { null }, listOf(100L, 500L), { sleeps += it }, sdkInt)

    private fun failureOf(block: () -> Unit): SecretBoxException {
        try {
            block()
        } catch (e: SecretBoxException) {
            return e
        }
        fail("expected a SecretBoxException")
        error("unreachable")
    }

    private fun kindOf(block: () -> Unit) = failureOf(block).kind

    /** The state of a keystore that really lost the main key: the canary is still there and works. */
    private fun mainLostCanaryAlive() {
        ks.aliasesDefault = Result.success(listOf("croft.test.canary"))
        ks.canaryDefault = Result.success(true)
    }

    @Test
    fun aKeyThatIsThereIsReturnedAtOnceWithNoWaitingAndNoCanaryCheck() {
        ks.getKeyDefault = Result.success(key)
        assertEquals(key, source().keyForOpening())
        assertEquals(key, source().keyForSealing())
        assertTrue(sleeps.isEmpty())
        assertTrue(ks.generated.isEmpty())
        assertEquals(0, ks.canaryCalls.get())
    }

    @Test
    fun aMainKeyThatIsMissingWhileTheCanaryIsListedAndUsableIsLostOnThreeSpacedChecks() {
        mainLostCanaryAlive()
        val e = failureOf { source().keyForOpening() }
        assertEquals(SecretBoxException.Kind.KEY_LOST, e.kind)
        assertFalse("on API 34 the evidence is definite", e.weak)
        assertEquals(3, ks.getKeyCalls.get())
        assertEquals(3, ks.aliasCalls.get())
        assertEquals(3, ks.canaryCalls.get())
        assertEquals(listOf(100L, 500L), sleeps)
        assertTrue(ks.generated.isEmpty() && ks.deleted.isEmpty())
    }

    @Test
    fun onApi30TheSameEvidenceIsWeak() {
        // keystore1 answers "nothing there" when its service call fails: the caller must see it persist.
        mainLostCanaryAlive()
        val e = failureOf { source(sdkInt = 30).keyForOpening() }
        assertEquals(SecretBoxException.Kind.KEY_LOST, e.kind)
        assertTrue(e.weak)
    }

    @Test
    fun anEmptyAliasListIsUnavailableNotALoss() {
        // getKey null and aliases() empty: exactly what keystore1 answers when the service fails.
        ks.aliasesDefault = Result.success(emptyList())
        val e = failureOf { source().keyForOpening() }
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, e.kind)
        assertTrue("persisting emptiness is tracked by the caller", e.weak)
        assertEquals(0, ks.canaryCalls.get())
    }

    @Test
    fun aFailingAliasListIsUnavailable() {
        ks.aliasesDefault = Result.failure(ProviderException("service down"))
        val e = failureOf { source().keyForOpening() }
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, e.kind)
        assertFalse(e.weak)
    }

    @Test
    fun aMissingCanaryMeansTheKeystoreIsNotAnsweringSoItIsUnavailable() {
        // The list has something but neither of our keys: a wiped keystore, or one that is not answering.
        ks.aliasesDefault = Result.success(listOf("some.other.alias"))
        val e = failureOf { source().keyForOpening() }
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, e.kind)
        assertTrue(e.weak)
        assertTrue(ks.generated.isEmpty() && ks.deleted.isEmpty())
    }

    @Test
    fun aCanaryThatIsListedButDoesNotWorkIsUnavailable() {
        ks.aliasesDefault = Result.success(listOf("croft.test.canary"))
        ks.canaryDefault = Result.failure(ProviderException("service down"))
        val e = failureOf { source().keyForOpening() }
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, e.kind)
        assertFalse(e.weak)
        ks.canaryDefault = Result.success(false)
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForOpening() })
    }

    @Test
    fun aNullKeyThatIsStillListedIsATransientFailureNotALoss() {
        // Android 8 to 11: getKey returns null when the service is unreachable; the alias is listed.
        ks.aliasesDefault = Result.success(listOf("croft.test", "croft.test.canary"))
        ks.canaryDefault = Result.success(true)
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForOpening() })
        assertEquals(3, ks.getKeyCalls.get())
    }

    @Test
    fun aNullKeyThatComesBackOnALaterTryIsUsed() {
        ks.getKeySteps += Result.success(null)
        ks.getKeySteps += Result.success(key)
        ks.aliasSteps += Result.success(listOf("croft.test.canary"))
        ks.canaryDefault = Result.success(true)
        assertEquals(key, source().keyForOpening())
        assertEquals(listOf(100L), sleeps)
    }

    @Test
    fun aMixedAnswerIsNotEvidence() {
        mainLostCanaryAlive()
        // lost, failing, lost: not three agreeing checks.
        ks.getKeySteps += Result.success(null)
        ks.getKeySteps += Result.failure(ProviderException("service down"))
        ks.getKeySteps += Result.success(null)
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().keyForOpening() })
    }

    @Test
    fun oneCanaryFailureAmongTheChecksKeepsItFromBeingALoss() {
        mainLostCanaryAlive()
        ks.canarySteps += Result.success(true)
        ks.canarySteps += Result.failure(ProviderException("busy"))
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
    fun aPermanentlyInvalidatedKeyIsLostAtOnceAndDefinite() {
        ks.getKeyDefault = Result.failure(UnrecoverableKeyException().initCause(KeyPermanentlyInvalidatedException()))
        val e = failureOf { source(sdkInt = 28).keyForOpening() }
        assertEquals(SecretBoxException.Kind.KEY_LOST, e.kind)
        assertFalse(e.weak)
        assertEquals(1, ks.getKeyCalls.get())
        assertTrue(sleeps.isEmpty())
    }

    @Test
    fun sealingNeverGeneratesAKeyWhateverTheKeystoreAnswers() {
        val states = listOf<() -> Unit>(
            { ks.aliasesDefault = Result.success(emptyList()) },
            { ks.aliasesDefault = Result.failure(ProviderException("down")) },
            { mainLostCanaryAlive() },
            { ks.aliasesDefault = Result.success(listOf("other")) },
            { ks.aliasesDefault = Result.success(listOf("croft.test")) },
            { ks.getKeyDefault = Result.failure(ProviderException("down")) },
            { ks.getKeyDefault = Result.failure(UnrecoverableKeyException().initCause(KeyPermanentlyInvalidatedException())) },
        )
        for (state in states) {
            ks.getKeyDefault = Result.success(null)
            ks.aliasesDefault = Result.success(emptyList())
            state()
            val kind = kindOf { source().keyForSealing() }
            assertTrue(kind == SecretBoxException.Kind.KEY_LOST || kind == SecretBoxException.Kind.UNAVAILABLE)
        }
        assertTrue("no key was generated", ks.generated.isEmpty())
        assertTrue("nothing was deleted", ks.deleted.isEmpty())
    }

    @Test
    fun createMakesTheCanaryAndThenTheMainKey() {
        assertNotNull(source().create())
        assertEquals(listOf("croft.test.canary", "croft.test"), ks.generated)
    }

    @Test
    fun aFailedCreationIsUnavailable() {
        ks.generateFails = ProviderException("no secure hardware")
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, kindOf { source().create() })
    }

    @Test
    fun discardRemovesBothKeys() {
        source().discard()
        assertEquals(listOf("croft.test", "croft.test.canary"), ks.deleted)
    }

    @Test
    fun theWaitsBetweenChecksHappenWithoutTheMonitorHeld() {
        mainLostCanaryAlive()
        lateinit var src: KeystoreKeySource
        var heldDuringASleep = false
        var slept = 0
        src = KeystoreKeySource(
            "croft.test", ks, KeystoreErrors(sdkInt = 34) { null }, listOf(100L, 500L),
            { slept++; heldDuringASleep = heldDuringASleep || Thread.holdsLock(src) },
        )
        kindOf { src.keyForOpening() }
        assertEquals(2, slept)
        assertFalse("the back-off sleeps outside the monitor", heldDuringASleep)
    }
}
