// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.example.croft.mail.reading.UnsubscribeInfoData
import com.example.croft.mail.reading.UnsubscribeProblem
import com.example.croft.mail.reading.UnsubscribeResultData
import com.example.croft.mail.reading.UnsubscribeWay
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** T37: S21's unsubscribe banner and S31: nothing is sent before the confirmation; the outcomes; a website opens only after S31. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class UnsubscribeTest {
    @get:Rule
    val rule = createComposeRule()

    private fun exists(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun textExists(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

    private fun info(way: UnsubscribeWay, older: Int = 0, already: Boolean = false) = UnsubscribeInfoData(
        way = way,
        host = if (way == UnsubscribeWay.EMAIL) null else "list.example",
        address = if (way == UnsubscribeWay.EMAIL) "leave@list.example" else null,
        sender = "Croft Dev List",
        olderCount = older,
        already = already,
        senderAddress = "list@croft.example",
        subject = if (way == UnsubscribeWay.EMAIL) "unsubscribe" else null,
        body = if (way == UnsubscribeWay.EMAIL) "" else null,
    )

    private fun fixture(info: UnsubscribeInfoData?): ReadingFixture = ReadingFixture().also { f ->
        info?.let { f.reading.unsubscribeInfos[NEWEST] = it }
    }

    private fun unsubscribeCalls(f: ReadingFixture) = f.reading.calls.filter { it.startsWith("unsubscribe:") }

    @Test
    fun noBannerWithoutAListHeader() {
        val f = fixture(null)
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        assertTrue(!exists(UnsubscribeTags.BANNER))
    }

    @Test
    fun theBannerAsksFirst_cancelSendsNothing_andConfirmingUnsubscribesWithTheOlderMail() {
        val f = fixture(info(UnsubscribeWay.ONE_CLICK, older = 14))
        f.show(rule)
        rule.waitForTag(UnsubscribeTags.BANNER)
        rule.onNodeWithTag(UnsubscribeTags.ACTION).performClick()
        rule.waitForTag(UnsubscribeTags.DIALOG)
        assertTrue("names the sender", textExists("Unsubscribe from Croft Dev List?"))
        assertTrue("says where the request goes", textExists("list.example"))
        rule.onNodeWithTag(UnsubscribeTags.CANCEL).performClick()
        rule.waitUntil(10_000) { !exists(UnsubscribeTags.DIALOG) }
        assertTrue("nothing sent", unsubscribeCalls(f).isEmpty())

        rule.onNodeWithTag(UnsubscribeTags.ACTION).performClick()
        rule.waitForTag(UnsubscribeTags.DIALOG)
        assertTrue("the delete names the address, not only the name", textExists("Also delete the 14 older messages from list@croft.example"))
        rule.onNodeWithTag(UnsubscribeTags.DELETE_OLDER).performClick()
        rule.onNodeWithTag(UnsubscribeTags.CONFIRM).performClick()
        rule.waitUntil(10_000) { unsubscribeCalls(f).isNotEmpty() }
        assertEquals(listOf("unsubscribe:$NEWEST:true"), unsubscribeCalls(f))
        rule.waitUntil(10_000) { textExists("Unsubscribed from Croft Dev List") }
        rule.waitUntil(10_000) { textExists("You unsubscribed from Croft Dev List") }
        assertTrue("no second offer", !exists(UnsubscribeTags.ACTION))
    }

    @Test
    fun theEmailWayNamesTheAddress_andAFailureIsSaid() {
        val f = fixture(info(UnsubscribeWay.EMAIL))
        f.reading.unsubscribeResult =
            UnsubscribeResultData(done = false, openUrl = null, problem = UnsubscribeProblem.CANNOT_SEND, deletedOps = emptyList())
        f.show(rule)
        rule.waitForTag(UnsubscribeTags.BANNER)
        rule.onNodeWithTag(UnsubscribeTags.ACTION).performClick()
        rule.waitForTag(UnsubscribeTags.DIALOG)
        assertTrue(textExists("leave@list.example"))
        assertTrue("the email is shown before it is sent", textExists("Subject: unsubscribe"))
        assertTrue("no delete offered without older mail", !exists(UnsubscribeTags.DELETE_OLDER))
        rule.onNodeWithTag(UnsubscribeTags.CONFIRM).performClick()
        rule.waitUntil(10_000) { textExists("Couldn't send the unsubscribe email") }
        rule.onNodeWithTag(UnsubscribeTags.ACTION).assertIsDisplayed()
    }

    @Test
    fun aWebsiteOpensInTheBrowserOnlyAfterTheConfirmation() {
        val f = fixture(info(UnsubscribeWay.WEBSITE))
        f.reading.unsubscribeResult =
            UnsubscribeResultData(done = false, openUrl = "https://list.example/manage", problem = null, deletedOps = emptyList())
        f.show(rule)
        rule.waitForTag(UnsubscribeTags.BANNER)
        rule.onNodeWithTag(UnsubscribeTags.ACTION).performClick()
        rule.waitForTag(UnsubscribeTags.DIALOG)
        assertTrue(f.links.isEmpty())
        rule.onNodeWithTag(UnsubscribeTags.CONFIRM).performClick()
        rule.waitUntil(10_000) { f.links.isNotEmpty() }
        assertEquals(listOf("https://list.example/manage"), f.links.toList())
    }
}
