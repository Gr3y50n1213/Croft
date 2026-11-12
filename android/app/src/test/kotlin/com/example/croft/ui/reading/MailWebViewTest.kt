// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.content.Context
import android.webkit.WebSettings
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.core.app.ApplicationProvider
import com.example.croft.mail.reading.LinkAction
import com.example.croft.ui.components.show
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * The one WebView of the app, which shows HTML mail: every flag of its settings, what its client does with links and with requests, and
 * what it is given. Email content is hostile, so these are security tests: a change to any flag fails here.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MailWebViewTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()

    @Test
    fun theSettingsAreExactlyTheListedOnes_whileRemoteContentIsBlocked() {
        val view = MailWebViewFactory.create(context)
        val s = view.settings
        assertFalse("JavaScript off", s.javaScriptEnabled)
        assertFalse("file access off", s.allowFileAccess)
        assertFalse("content access off", s.allowContentAccess)
        assertFalse(s.allowFileAccessFromFileURLs)
        assertFalse(s.allowUniversalAccessFromFileURLs)
        assertTrue("no network while remote content is blocked", s.blockNetworkLoads)
        assertTrue("no pictures from the network while blocked", s.blockNetworkImage)
        assertTrue("Safe Browsing on", s.safeBrowsingEnabled)
        // WebSettings has no getter for this one; Robolectric's own settings object does.
        assertFalse("no geolocation", (s as org.robolectric.fakes.RoboWebSettings).geolocationEnabled)
        assertFalse("no saved forms", s.saveFormData)
        assertFalse("no web storage", s.domStorageEnabled)
        assertFalse("no databases", s.databaseEnabled)
        assertFalse(s.javaScriptCanOpenWindowsAutomatically)
        assertFalse(s.supportMultipleWindows())
        assertEquals(WebSettings.MIXED_CONTENT_NEVER_ALLOW, s.mixedContentMode)
        assertEquals(WebSettings.LOAD_NO_CACHE, s.cacheMode)
        assertEquals("the email's own colours are never darkened", WebSettings.FORCE_DARK_OFF, s.forceDark)
        assertFalse("no on-screen zoom buttons", s.displayZoomControls)
    }

    @Test
    fun allowingRemoteContentOnlyOpensTheNetwork_nothingElseChanges() {
        val view = MailWebViewFactory.create(context)
        MailWebViewFactory.apply(view.settings, remoteAllowed = true, fontScale = 1f)
        val s = view.settings
        assertFalse(s.blockNetworkLoads)
        assertFalse(s.blockNetworkImage)
        assertFalse(s.javaScriptEnabled)
        assertFalse(s.allowFileAccess)
        assertFalse(s.allowContentAccess)
        assertTrue(s.safeBrowsingEnabled)
        assertFalse(s.domStorageEnabled)
        assertFalse(s.databaseEnabled)
        assertFalse(s.saveFormData)
        assertEquals(WebSettings.MIXED_CONTENT_NEVER_ALLOW, s.mixedContentMode)
        // And back: blocking again closes it.
        MailWebViewFactory.apply(view.settings, remoteAllowed = false, fontScale = 1f)
        assertTrue(s.blockNetworkLoads && s.blockNetworkImage)
    }

    @Test
    fun theMessageFollowsTheTextSize() {
        val view = MailWebViewFactory.create(context)
        MailWebViewFactory.apply(view.settings, false, 1.0f)
        assertEquals(100, view.settings.textZoom)
        MailWebViewFactory.apply(view.settings, false, 2.0f)
        assertEquals(200, view.settings.textZoom)
        MailWebViewFactory.apply(view.settings, false, 9f)
        assertEquals("capped", 300, view.settings.textZoom)
    }

    @Test
    fun noJavascriptInterfaceIsAdded_andNoChromeClientOrDownloadListener() {
        val view = MailWebViewFactory.create(context)
        val shadow = shadowOf(view)
        val field = shadow.javaClass.getDeclaredField("javascriptInterfaces").also { it.isAccessible = true }
        assertTrue("no object is exposed to the page", (field.get(shadow) as Map<*, *>).isEmpty())
        assertNull(shadow.webChromeClient)
        assertNull(shadow.downloadListener)
        assertTrue(shadow.webViewClient is MailWebViewClient)
    }

    @Test
    fun theSourcesNeverAddAScriptBridge_orLoadAFileOrAUrlIntoThePage() {
        val problems = mutableListOf<String>()
        File("src/main/kotlin").walkTopDown().filter { it.isFile && it.extension == "kt" }.forEach { f ->
            f.readLines().forEachIndexed { i, line ->
                val code = line.substringBefore("//").trim()
                if (code.startsWith("*") || code.startsWith("/*")) return@forEachIndexed
                if (code.contains("addJavascriptInterface")) problems += "${f.path}:${i + 1} addJavascriptInterface"
                if (code.contains("javaScriptEnabled = true")) problems += "${f.path}:${i + 1} JavaScript on"
                if (code.contains("setJavaScriptEnabled(true)")) problems += "${f.path}:${i + 1} JavaScript on"
                if (code.contains("allowFileAccess = true") || code.contains("allowContentAccess = true")) problems += "${f.path}:${i + 1} file or content access on"
                if (f.path.contains("/ui/reading/") || f.path.contains("/mail/reading/")) {
                    if (code.contains("Uri.fromFile") || code.contains("\"file://")) problems += "${f.path}:${i + 1} a file:// URI"
                    if (code.contains(".loadUrl(") || code.contains("evaluateJavascript")) problems += "${f.path}:${i + 1} loads a URL or runs a script in the page"
                    if (code.contains("setWebChromeClient")) problems += "${f.path}:${i + 1} a chrome client"
                }
            }
        }
        assertTrue(problems.joinToString("\n"), problems.isEmpty())
    }

    // ---- The client ----

    private val tapped = listOf(
        "https://example.org/a", "http://example.org/b", "mailto:ann@example.org", "javascript:alert(1)", "file:///data/data/x/databases/croft.db",
        "content://com.example.croft.attachments/shared_attachments/x.pdf", "intent://scan/#Intent;scheme=zxing;end", "tel:+15550100", "data:text/html,hi", "", "  ", "ftp://example.org/x",
    )

    @Test
    fun aTappedLinkIsNeverLoadedInThePage_andOnlyWebAndMailtoReachTheScreen() {
        val view = MailWebViewFactory.create(context)
        val client = view.tag as MailWebViewClient
        val seen = mutableListOf<LinkAction>()
        client.onLink = { seen += it }
        for (url in tapped) {
            assertTrue("always true: $url", client.shouldOverrideUrlLoading(view, FakeRequest(url)))
            @Suppress("DEPRECATION")
            assertTrue(client.shouldOverrideUrlLoading(view, url))
        }
        assertNull("nothing was loaded in the page", shadowOf(view).lastLoadedUrl)
        // Two calls per link.
        val kinds = seen.map { it::class.simpleName }
        assertEquals(2, kinds.count { it == "Mail" })
        assertEquals(4, kinds.count { it == "Web" })
        assertEquals((tapped.size - 3) * 2, kinds.count { it == "Ignore" })
        assertEquals(setOf("https://example.org/a", "http://example.org/b"), seen.filterIsInstance<LinkAction.Web>().map { it.url }.toSet())
    }

    @Test
    fun whileRemoteContentIsBlockedEveryRequestGetsAnEmptyAnswer() {
        val view = MailWebViewFactory.create(context)
        val client = view.tag as MailWebViewClient
        client.remoteAllowed = false
        for (url in listOf("https://img.example.org/p.png", "http://img.example.org/p.png", "file:///etc/hosts", "content://x/y", "ftp://h/f", "https://tracker.example/pixel.gif?id=1")) {
            val r = client.shouldInterceptRequest(view, FakeRequest(url))
            assertNotNull("answered, not loaded: $url", r)
            assertEquals(403, r!!.statusCode)
            assertEquals("empty", -1, r.data.read())
        }
        assertEquals(6, client.blockedRequests)
        @Suppress("DEPRECATION")
        assertNotNull(client.shouldInterceptRequest(view, "https://img.example.org/p.png"))
        // The core's own inline pictures never leave the device.
        assertNull(client.shouldInterceptRequest(view, FakeRequest("data:image/png;base64,iVBORw0KGgo=")))
        assertNull(client.shouldInterceptRequest(view, FakeRequest("about:blank")))
    }

    @Test
    fun whenRemoteContentIsAllowedOnlyHttpsGoesThrough() {
        val view = MailWebViewFactory.create(context)
        val client = view.tag as MailWebViewClient
        client.remoteAllowed = true
        assertNull(client.shouldInterceptRequest(view, FakeRequest("https://img.example.org/chart.png")))
        for (url in listOf("http://img.example.org/chart.png", "file:///etc/hosts", "content://x/y", "ftp://h/f", "ws://h/socket", "jar:file:///x.jar!/y")) {
            assertNotNull("refused: $url", client.shouldInterceptRequest(view, FakeRequest(url)))
        }
        assertNull(client.shouldInterceptRequest(view, FakeRequest("data:image/gif;base64,R0lGODlhAQABAAAAACw=")))
    }

    // ---- What the screen gives it ----

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun theHtmlIsLoadedWithNoBaseUrlAndNothingElse() {
        val html = "<p>Hello</p><img src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\">"
        rule.show { HtmlBody(1, html, remoteAllowed = false, onLink = {}) }
        rule.waitForTag(BodyTags.html(1))
        rule.waitForIdle()
        val view = rule.activity.webView()
        assertNotNull(view)
        val load = shadowOf(view).lastLoadDataWithBaseURL
        assertNotNull(load)
        assertNull("no base URL", load.baseUrl)
        assertEquals(html, load.data)
        assertEquals("text/html", load.mimeType)
        assertEquals("utf-8", load.encoding)
        assertNull(load.historyUrl)
        assertTrue((view!!.tag as MailWebViewClient).remoteAllowed.not())
        assertTrue(view.settings.blockNetworkLoads)
        rule.onNodeWithTag(BodyTags.html(1)).assertExists()
    }
}
