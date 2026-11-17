// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import com.example.croft.CroftRobolectricRunner
import com.example.croft.mail.reading.BodySecurity
import com.example.croft.mail.reading.BodySecurity.DecryptIssue
import com.example.croft.mail.reading.BodySecurity.Encryption
import com.example.croft.mail.reading.BodySecurity.Signature
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.show
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** S56 (T42b): what the banner says for each state. Made-up keys and addresses. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class SecurityBannerTest {
    @get:Rule
    val rule = createComposeRule()

    private var security by mutableStateOf(BodySecurity())
    private val key = "3F2A91C47B10E5D28A6C40B19F7E62C3D8150A4B"

    private fun show() {
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) { SecurityBanner(7, security) }
    }

    private fun says(text: String) = rule.onNodeWithText(text, substring = true).assertExists()

    @Test
    fun encryptedAndSignedNamesTheSignerAndTheKey() {
        security = BodySecurity(Encryption.DECRYPTED, null, Signature.VALID, key, "mira@example.org")
        show()
        says("Encrypted and signed")
        // The last eight characters of the key, in two groups (as S54 shows a key).
        says("Signed by mira@example.org, key D815 0A4B")
    }

    @Test
    fun encryptedWithoutASignatureSaysSoAndThatNothingIsKept() {
        security = BodySecurity(Encryption.DECRYPTED)
        show()
        says("Encrypted")
        says("Nothing of it is kept unencrypted")
    }

    @Test
    fun aKeyFromTheirOwnMailIsNeverShownAsVerified() {
        security = BodySecurity(Encryption.DECRYPTED, null, Signature.UNCONFIRMED, key, "dan@example.net")
        show()
        says("Key not confirmed")
        says("not proof the message is from them")
        assertTrue(rule.onAllNodes(androidx.compose.ui.test.hasText("Encrypted and signed")).fetchSemanticsNodes().isEmpty())
        assertTrue(rule.onAllNodes(hasTestTag(SecurityTags.signature(7))).fetchSemanticsNodes().isNotEmpty())
    }

    @Test
    fun aSignatureThatDoesNotCheckSaysSo() {
        security = BodySecurity(signature = Signature.NOT_VERIFIED, signerKey = key)
        show()
        says("Signature not verified")
    }

    @Test
    fun aPlainSignedMessageNamesItsSigner() {
        security = BodySecurity(signature = Signature.VALID, signerKey = key, signer = "mira@example.org")
        show()
        says("Signed")
        says("Signed by mira@example.org")
    }

    @Test
    fun eachDecryptFailureSaysWhyAndWhatFixesIt() {
        security = BodySecurity(Encryption.FAILED, DecryptIssue.NOT_FOR_THIS_KEY)
        show()
        says("Could not decrypt this message")
        says("written for a key that is not on this phone")
        rule.runOnIdle { security = BodySecurity(Encryption.FAILED, DecryptIssue.NO_OWN_KEY) }
        says("Import it from Settings, Encryption keys")
        rule.runOnIdle { security = BodySecurity(Encryption.FAILED, DecryptIssue.DAMAGED) }
        says("damaged")
        rule.runOnIdle { security = BodySecurity(Encryption.FAILED, DecryptIssue.TOO_LARGE) }
        says("too large")
    }
}
