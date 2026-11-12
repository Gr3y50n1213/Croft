// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.reading.BodyData
import com.example.croft.mail.reading.ConversationData
import com.example.croft.mail.reading.ConversationMessageData
import com.example.croft.mail.reading.LinkAction
import com.example.croft.mail.reading.LinkRules
import com.example.croft.mail.reading.MailAddressData
import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.mail.reading.ReadingRules
import com.example.croft.mail.reading.RemoteImages
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The rules of the reading screens that need no screen: which links may open, which requests may load, what a plain-text link is, who is a reply-all recipient. */
class ReadingRulesTest {
    // ---- Links ----

    @Test
    fun onlyWebAndMailtoLinksAreAskedAbout_everythingElseIsIgnored() {
        assertEquals(LinkAction.Web("https://example.org/a?b=1#c"), LinkRules.classify("https://example.org/a?b=1#c"))
        assertEquals(LinkAction.Web("http://example.org"), LinkRules.classify("  http://example.org  "))
        assertEquals(LinkAction.Web("HTTPS://EXAMPLE.ORG/"), LinkRules.classify("HTTPS://EXAMPLE.ORG/"))
        assertEquals(LinkAction.Mail("mailto:ann@example.org"), LinkRules.classify("mailto:ann@example.org"))
        for (bad in listOf(
            null, "", " ", "javascript:alert(1)", "JaVaScRiPt:alert(1)", "file:///etc/passwd", "content://x/y", "intent://x#Intent;end", "tel:+1555", "sms:1",
            "data:text/html,x", "ftp://example.org", "//example.org/x", "example.org", "/relative", "https://", "https:///path", "http://\u0000evil.example",
            "https://example.org/\nSet-Cookie: x", "market://details?id=x", "blob:https://x/y", "vbscript:x",
        )) assertEquals("ignored: $bad", LinkAction.Ignore, LinkRules.classify(bad))
        assertEquals("far too long", LinkAction.Ignore, LinkRules.classify("https://example.org/" + "a".repeat(9_000)))
    }

    @Test
    fun theHostIsTheOneTheUrlReallyGoesTo_notTheOneAUserInfoPartPretendsTo() {
        assertEquals("example.org", LinkRules.hostOf("https://example.org/path"))
        assertEquals("example.org", LinkRules.hostOf("https://example.org:8443/path?x=1"))
        assertEquals("evil.example", LinkRules.hostOf("https://paypal.com@evil.example/login"))
        assertEquals("evil.example", LinkRules.hostOf("https://paypal.com:secret@evil.example:443/"))
        assertEquals("::1", LinkRules.hostOf("http://[::1]:8080/x"))
        assertEquals("a.example", LinkRules.hostOf("https://a.example?x=1"))
        assertNull(LinkRules.hostOf("https://"))
        assertNull(LinkRules.hostOf("mailto:a@b.example"))
    }

    @Test
    fun charactersThatCouldDisguiseAnAddressAreWrittenOut() {
        assertEquals("https://example.org/path", LinkRules.displayable("https://example.org/path"))
        assertEquals("https://example.org/\\u202Efdp.exe", LinkRules.displayable("https://example.org/‮fdp.exe"))
        assertEquals("https://exa\\u200Bmple.org", LinkRules.displayable("https://exa​mple.org"))
        assertEquals("a\\u0000b", LinkRules.displayable("a\u0000b"))
        assertEquals("https://münchen.example/ü", LinkRules.displayable("https://münchen.example/ü"))
    }

    @Test
    fun linksInPlainTextAreFoundWithoutTheirSentencePunctuation() {
        val text = "See https://example.org/a, or (http://example.org/b). Mail mailto:team@example.org?subject=Hi! Not javascript:alert(1) or www.example.org"
        val found = LinkRules.linkSpans(text)
        assertEquals(
            listOf("https://example.org/a", "http://example.org/b", "mailto:team@example.org?subject=Hi"),
            found.map { text.substring(it.start, it.end) },
        )
        assertEquals(found.map { it.url }, found.map { text.substring(it.start, it.end) })
        assertTrue(LinkRules.linkSpans("no links here").isEmpty())
        assertEquals(1, LinkRules.linkSpans("HTTPS://EXAMPLE.ORG").size)
    }

    // ---- Requests ----

    @Test
    fun whileRemoteContentIsBlockedNothingMayLoad_andWhenAllowedOnlyHttps() {
        for (url in listOf("https://img.example.org/p.png", "http://img.example.org/p.png", "file:///x", "content://a/b", "ftp://h", "ws://h", "")) {
            assertFalse("blocked: $url", LinkRules.requestAllowed(url, remoteAllowed = false))
        }
        assertFalse(LinkRules.requestAllowed(null, remoteAllowed = true))
        assertTrue(LinkRules.requestAllowed("https://img.example.org/p.png", remoteAllowed = true))
        for (url in listOf("http://img.example.org/p.png", "file:///x", "content://a/b", "ftp://h", "ws://h", "jar:file:///x!/y", "blob:https://x/y")) {
            assertFalse("not https: $url", LinkRules.requestAllowed(url, remoteAllowed = true))
        }
        for (allowed in listOf(false, true)) {
            assertTrue("the core's own pictures never leave the device", LinkRules.requestAllowed("data:image/png;base64,AAAA", allowed))
            assertTrue(LinkRules.requestAllowed("about:blank", allowed))
        }
    }

    // ---- Reply all and the actions' targets ----

    private fun message(id: Long, folder: Long = 10, sender: String? = "ann@example.org", to: List<String> = emptyList(), cc: List<String> = emptyList()) = ConversationMessageData(
        id = id, account = 1, folder = folder, folderRole = FolderRole.INBOX, senderName = "Ann", senderAddress = sender,
        to = to.map { MailAddressData(null, it) }, cc = cc.map { MailAddressData(null, it) }, dateMs = id, subject = "S", snippet = "", seen = false,
        flagged = false, answered = false, hasAttachments = false, pending = false, bodyStored = true, remoteAllowed = false,
    )

    @Test
    fun replyAllNeedsMoreThanOneOtherPerson_meNotCounted_norRepeats() {
        val me = "me@example.org"
        assertFalse("only the sender", ReadingRules.showReplyAll(message(1, to = listOf(me)), me))
        assertFalse("the sender twice", ReadingRules.showReplyAll(message(1, to = listOf(me, "ANN@example.org")), me))
        assertTrue("one more in To", ReadingRules.showReplyAll(message(1, to = listOf(me, "bo@example.org")), me))
        assertTrue("one more in Cc", ReadingRules.showReplyAll(message(1, to = listOf(me), cc = listOf("cy@example.org")), me))
        assertTrue("my address is not known: nobody is left out", ReadingRules.showReplyAll(message(1, to = listOf("a@x.org")), null))
        assertFalse("recipients not loaded yet", ReadingRules.showReplyAll(message(1), me))
        assertFalse(ReadingRules.showReplyAll(message(1, sender = null, to = listOf(me)), me))
    }

    @Test
    fun anActionChangesTheMessagesOfTheFolderThatWasOpened_theOpenedOneFirst() {
        val c = ConversationData(1, 3, listOf(message(1, folder = 10), message(2, folder = 99), message(3, folder = 10), message(4, folder = 10)))
        assertEquals(listOf(3L, 1L, 4L), ReadingRules.targets(c).map { it.id })
        assertEquals(emptyList<Long>(), ReadingRules.targets(ConversationData(1, 7, listOf(message(1)))).map { it.id })
        assertEquals(setOf(3L, 4L), c.initiallyExpanded)
        val row = ReadingRules.rowOf(message(2, folder = 99), localOnly = true)
        assertEquals(2L, row.id)
        assertEquals(99L, row.folder)
        assertTrue(row.localOnly)
    }

    // ---- Settings ----

    @Test
    fun theSettingsKeysRoundTripAndUnknownKeysAreTheDefaults() {
        for (r in RemoteImages.entries) assertEquals(r, RemoteImages.fromKey(r.key))
        for (m in MarkReadDelay.entries) assertEquals(m, MarkReadDelay.fromKey(m.key))
        assertEquals(RemoteImages.ASK, RemoteImages.fromKey("everything"))
        assertEquals(RemoteImages.ASK, RemoteImages.fromKey(null))
        assertEquals(MarkReadDelay.AFTER_2_SECONDS, MarkReadDelay.fromKey("now"))
        assertEquals(listOf(0L, 2_000L, null), MarkReadDelay.entries.map { it.delayMs })
    }

    @Test
    fun aBodyThatCouldNotBeShownAsHtmlIsRecognised_andInlinePicturesGetNoChip() {
        val attachments = listOf(
            com.example.croft.mail.reading.AttachmentData(1, "a.pdf", "application/pdf", 10, shownInline = false, blocked = false),
            com.example.croft.mail.reading.AttachmentData(2, "logo.png", "image/png", 10, shownInline = true, blocked = false),
        )
        val complex = BodyData(1, "text", null, false, 0, 0, false, truncated = true, attachments = attachments)
        assertTrue(complex.shownAsPlainText)
        assertEquals(listOf(1L), complex.chips.map { it.id })
        assertFalse(complex.copy(truncated = false).shownAsPlainText)
        assertFalse(complex.copy(html = "<p>x</p>").shownAsPlainText)
        // Nothing printed holds message text.
        assertFalse(complex.toString().contains("text"))
    }
}
