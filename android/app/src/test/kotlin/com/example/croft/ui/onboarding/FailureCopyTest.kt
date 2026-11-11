// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import com.example.croft.R
import com.example.croft.mail.onboarding.OnboardingFailure
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.DiscoverException
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.ProbeFailureReason
import uniffi.croft_ffi.StorageReason
import uniffi.croft_ffi.VerifyFailure
import uniffi.croft_ffi.discoverErrorMessageKey
import uniffi.croft_ffi.verifyFailureMessageKey

/**
 * Every discovery and sign-in error the core can return has its own plain-language text. The test walks the keys three ways, so a new
 * error in the core cannot slip through: (1) every translation key written in `core/ffi/src/error.rs`, (2) every variant of
 * the generated bindings (`DiscoverException`, `VerifyFailure` with each provider, `CoreException`), turned into its key by the
 * core's own key functions, and (3) every probe reason. Each key must have a string resource of the right name, with text, that
 * shows no raw code. Plain JVM test: it reads the Rust source and `strings_onboarding.xml` as files.
 */
class FailureCopyTest {
    private val repo: File = generateSequence(File("").absoluteFile) { it.parentFile }.first { File(it, "core/ffi/src/error.rs").isFile }
    private val strings: Map<String, String> = Regex("""<string name="(\w+)">(.*?)</string>""", RegexOption.DOT_MATCHES_ALL)
        .findAll(File("src/main/res/values/strings_onboarding.xml").readText())
        .associate { it.groupValues[1] to it.groupValues[2] }

    /** Every `discover.*` and `verify.*` key written in the core's error file. */
    private val rustKeys: Set<String> = Regex(""""((?:discover|verify)\.[a-z_.]+)"""").findAll(File(repo, "core/ffi/src/error.rs").readText())
        .map { it.groupValues[1] }.toSet()

    private fun assertHasText(key: String) {
        val res = FailureCopy.resFor(key)
        assertNotNull("no string for the core's key '$key' (add it to FailureCopy and strings_onboarding.xml)", res)
        val name = FailureCopy.resourceName(key)
        assertEquals("'$key' must use the resource $name", R.string::class.java.getField(name).getInt(null), res)
        val text = strings[name]
        assertNotNull("strings_onboarding.xml has no <string name=\"$name\">", text)
        assertTrue("$name is empty", text!!.isNotBlank())
        assertTrue("$name shows a raw code: $text", !text.contains(key) && !Regex("""Exception|0x[0-9a-f]+|\b[a-z]+_[a-z_]+\b""").containsMatchIn(text.replace(Regex("""%\d\$[sd]"""), "")))
    }

    @Test
    fun everyKeyInTheCoresErrorFileHasAString() {
        // The file is read as text, so make sure the pattern found the keys and not nothing.
        assertTrue("found only $rustKeys", rustKeys.size >= 18)
        rustKeys.forEach(::assertHasText)
    }

    @Test
    fun everyVariantOfTheGeneratedBindingsHasAString_andTheKeysMatchTheRustFile() {
        val discover = listOf(
            DiscoverException.InvalidAddress(), DiscoverException.Offline(), DiscoverException.NothingFound(),
            DiscoverException.ResolverUnavailable(), DiscoverException.Internal(),
        )
        val verify = listOf(
            VerifyFailure.WrongPassword(), VerifyFailure.UntrustedCertificate("AA"), VerifyFailure.CertificateChanged("AA"),
            VerifyFailure.ConnectionIntercepted(), VerifyFailure.CertificateInvalid(), VerifyFailure.CannotConnect(),
            VerifyFailure.NoSecureConnection(), VerifyFailure.ServerProblem(), VerifyFailure.JmapOnly(), VerifyFailure.BadSettings(),
            VerifyFailure.Internal(),
        ) + KnownProvider.entries.map { VerifyFailure.NeedsAppPasswordOrOAuth(it) }
        // Nothing was forgotten: the classes of the bindings are exactly the ones listed.
        assertEquals(DiscoverException::class.java.declaredClasses.count { DiscoverException::class.java.isAssignableFrom(it) }, discover.size)
        assertEquals(VerifyFailure::class.java.declaredClasses.count { VerifyFailure::class.java.isAssignableFrom(it) }, verify.map { it.javaClass }.distinct().size)

        // The app's own table of keys equals the core's key functions, variant by variant.
        discover.forEach { assertEquals(discoverErrorMessageKey(it), OnboardingFailure.Discover(it, "mail.example.org").key) }
        verify.forEach { assertEquals(verifyFailureMessageKey(it), OnboardingFailure.Verify(it, "mail.example.org").key) }
        val keys = discover.map { OnboardingFailure.Discover(it, "mail.example.org").key } + verify.map { OnboardingFailure.Verify(it, "mail.example.org").key }
        keys.forEach(::assertHasText)
        assertEquals("the bindings and the Rust file name the same keys", rustKeys, keys.toSet())
        // One text per variant: no two variants share a message.
        assertEquals(keys.size, keys.distinct().size)
        assertEquals(keys.size, keys.map { strings.getValue(FailureCopy.resourceName(it)) }.distinct().size)
    }

    @Test
    fun everyErrorOfAddAccountHasAString() {
        val all = CoreException::class.java.declaredClasses.filter { CoreException::class.java.isAssignableFrom(it) }
        assertTrue("found ${all.size} CoreException variants", all.size >= 12)
        for (c in all) {
            val e = if (c == CoreException.Storage::class.java) CoreException.Storage(StorageReason.DATABASE) else c.getDeclaredConstructor().newInstance() as CoreException
            val key = OnboardingFailure.addKey(e)
            assertTrue("$key is not an app key", key in OnboardingFailure.appKeys)
            assertHasText(key)
        }
        OnboardingFailure.appKeys.forEach(::assertHasText)
        assertEquals(setOf(OnboardingFailure.KEY_STORAGE, OnboardingFailure.KEY_UNEXPECTED), OnboardingFailure.appKeys.filter { it.startsWith("app.") }.toSet())
    }

    @Test
    fun everyReasonAProbeCanGiveHasAString() {
        for (reason in ProbeFailureReason.entries) {
            val res = FailureCopy.probeReasonRes(reason)
            val name = R.string::class.java.fields.first { it.getInt(null) == res }.name
            assertTrue("$name has no text", strings.getValue(name).isNotBlank())
        }
    }
}
