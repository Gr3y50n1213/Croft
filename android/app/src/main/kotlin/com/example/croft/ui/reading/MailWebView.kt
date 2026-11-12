// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.annotation.SuppressLint
import android.content.Context
import android.net.http.SslError
import android.os.Build
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.RenderProcessGoneDetail
import android.webkit.SslErrorHandler
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import com.example.croft.mail.reading.LinkAction
import com.example.croft.mail.reading.LinkRules
import java.io.ByteArrayInputStream

/**
 * The one WebView of the app, for HTML mail only, and the rules it runs under. Email content is hostile, so these are security rules,
 * not style (docs/store/READINESS.md, `core/ffi/README.md` "How the app renders a message"):
 *
 * - It is given **`html_sanitised` and nothing else**, with `loadDataWithBaseURL(null, html, "text/html", "utf-8", null)`.
 * - JavaScript, file access, content access, web storage, databases, geolocation and form saving are off. There is no
 *   `addJavascriptInterface`, no `WebChromeClient`, no download listener.
 * - While remote content is blocked the page has no network at all (`blockNetworkLoads`, `blockNetworkImage`, and
 *   [MailWebViewClient.shouldInterceptRequest] answers every request with an empty response). When it is allowed, only `https`.
 * - A tap on a link never loads in the page: [MailWebViewClient.shouldOverrideUrlLoading] always returns true and hands the link to
 *   [onLink], which asks the person first ([LinkRules]).
 * - Dark theme: the email's colours are not recoloured. The screen puts the page on a light card instead (docs/UX.md: "complex HTML
 *   mail shows on a light card instead of being recoloured badly"), so every message reads as its sender drew it.
 */
object MailWebViewFactory {
    /** A new hardened WebView with its client. Throws when the device has no WebView (the screen then shows the text version). */
    fun create(context: Context): WebView {
        val view = WebView(context)
        view.layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        val client = MailWebViewClient()
        view.webViewClient = client
        view.tag = client
        // The parent column scrolls: the page itself neither scrolls vertically nor shows a scroll bar or an overscroll glow.
        view.isVerticalScrollBarEnabled = false
        view.overScrollMode = View.OVER_SCROLL_NEVER
        view.isLongClickable = true
        view.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        apply(view.settings, remoteAllowed = false, fontScale = 1f)
        try {
            // No cookies for any request the page makes, and none for the third parties of a remote picture.
            CookieManager.getInstance().setAcceptThirdPartyCookies(view, false)
        } catch (e: RuntimeException) {
            // No cookie manager on this device: nothing to switch off.
        }
        return view
    }

    /**
     * The settings of the page. [remoteAllowed] false: the network is blocked outright. [fontScale] is the combined text size (system
     * and S52), applied as the page's text zoom so that the message follows the person's text size.
     */
    @SuppressLint("SetJavaScriptEnabled")
    @Suppress("DEPRECATION")
    fun apply(s: WebSettings, remoteAllowed: Boolean, fontScale: Float) {
        s.javaScriptEnabled = false
        s.allowFileAccess = false
        s.allowContentAccess = false
        s.allowFileAccessFromFileURLs = false
        s.allowUniversalAccessFromFileURLs = false
        s.blockNetworkLoads = !remoteAllowed
        s.blockNetworkImage = !remoteAllowed
        s.safeBrowsingEnabled = true
        s.setGeolocationEnabled(false)
        s.saveFormData = false
        s.domStorageEnabled = false
        s.databaseEnabled = false
        s.javaScriptCanOpenWindowsAutomatically = false
        s.setSupportMultipleWindows(false)
        s.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
        s.cacheMode = WebSettings.LOAD_NO_CACHE
        s.mediaPlaybackRequiresUserGesture = true
        // The email's own colours: never darkened by the system's algorithmic darkening (API 29 to 32; later versions are off by default).
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) s.forceDark = WebSettings.FORCE_DARK_OFF
        // Fit the page to the width of the screen; pinch zoom is on for small print and the on-screen zoom buttons are off.
        s.useWideViewPort = true
        s.loadWithOverviewMode = true
        s.setSupportZoom(true)
        s.builtInZoomControls = true
        s.displayZoomControls = false
        s.textZoom = (fontScale * PERCENT).toInt().coerceIn(MIN_TEXT_ZOOM, MAX_TEXT_ZOOM)
        s.defaultTextEncodingName = "utf-8"
    }

    private const val PERCENT = 100
    private const val MIN_TEXT_ZOOM = 50
    private const val MAX_TEXT_ZOOM = 300
}

/**
 * The client of the message page. It owns three rules: links are never followed inside the page, every request is checked against
 * [LinkRules.requestAllowed] (an empty answer for anything that may not load), and a crashed renderer does not take the app with it.
 */
class MailWebViewClient : WebViewClient() {
    /** Remote content was allowed for the page now shown. Set by the screen before it loads the HTML. */
    @Volatile var remoteAllowed: Boolean = false

    /** Called with a tapped link. The screen shows the real destination and opens it only on the person's confirmation. */
    @Volatile var onLink: (LinkAction) -> Unit = {}

    /** The HTML and the remote flag of what the page shows now, so that an unchanged body is not loaded again. */
    internal var shown: Pair<String, Boolean>? = null

    /** How many requests the page made that were refused (a count, for tests: no address is kept). */
    @Volatile var blockedRequests: Int = 0
        private set

    override fun shouldOverrideUrlLoading(view: WebView?, request: WebResourceRequest?): Boolean {
        route(request?.url?.toString())
        return true
    }

    @Deprecated("Called on API 23 and older; kept so that no path loads a link in the page.")
    override fun shouldOverrideUrlLoading(view: WebView?, url: String?): Boolean {
        route(url)
        return true
    }

    override fun shouldInterceptRequest(view: WebView?, request: WebResourceRequest?): WebResourceResponse? = intercept(request?.url?.toString())

    @Deprecated("Called on API 20 and older; kept so that no path loads without the check.")
    override fun shouldInterceptRequest(view: WebView?, url: String?): WebResourceResponse? = intercept(url)

    override fun onRenderProcessGone(view: WebView?, detail: RenderProcessGoneDetail?): Boolean = true

    override fun onReceivedSslError(view: WebView?, handler: SslErrorHandler?, error: SslError?) {
        handler?.cancel()
    }

    private fun route(url: String?) {
        onLink(LinkRules.classify(url))
    }

    /** `null` lets the page load the request (a picture of its own, over `https`, after the person allowed remote content); anything else is answered with nothing. */
    private fun intercept(url: String?): WebResourceResponse? {
        if (LinkRules.requestAllowed(url, remoteAllowed)) return null
        blockedRequests++
        return emptyResponse()
    }

    private fun emptyResponse() = WebResourceResponse("text/plain", "utf-8", FORBIDDEN, "Blocked", emptyMap(), ByteArrayInputStream(ByteArray(0)))

    private companion object {
        const val FORBIDDEN = 403
    }
}
