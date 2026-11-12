// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

/** What a tap on a link in a message does. */
sealed interface LinkAction {
    /** A web link: shown to the person with its real destination, then opened in the browser (chooser) on confirm. */
    data class Web(val url: String) : LinkAction

    /** A `mailto:` link: shown, then opened in Croft's own compose on confirm. */
    data class Mail(val url: String) : LinkAction

    /** Anything else (`javascript:`, `file:`, `content:`, `intent:`, `tel:`, `data:`, a link with no scheme...): nothing happens. */
    data object Ignore : LinkAction
}

/**
 * The rules for links and for the requests of a message's page. Pure functions, so they can be tested without a WebView.
 *
 * Email content is hostile: a link is never followed inside the message's page, and what it opens is decided here, from the URL as the
 * HTML says it, never from the text that is shown for the link.
 */
object LinkRules {
    private val scheme = Regex("""^([A-Za-z][A-Za-z0-9+.\-]*):""")
    private const val MAX_URL = 8_192

    /** What a tap on [url] does. */
    fun classify(url: String?): LinkAction {
        val text = url?.trim().orEmpty()
        if (text.isEmpty() || text.length > MAX_URL) return LinkAction.Ignore
        if (text.any { it.isISOControl() }) return LinkAction.Ignore
        return when (schemeOf(text)) {
            "http", "https" -> if (hostOf(text).isNullOrEmpty()) LinkAction.Ignore else LinkAction.Web(text)
            "mailto" -> LinkAction.Mail(text)
            else -> LinkAction.Ignore
        }
    }

    /** The scheme of [url] in lower case, or null. */
    fun schemeOf(url: String): String? = scheme.find(url)?.groupValues?.get(1)?.lowercase()

    /** The host of a web link, or null when there is none. */
    fun hostOf(url: String): String? {
        val rest = url.substringAfter("://", missingDelimiterValue = "")
        if (rest.isEmpty()) return null
        val authority = rest.substringBefore('/').substringBefore('?').substringBefore('#')
        val hostPort = authority.substringAfterLast('@')
        val host = if (hostPort.startsWith("[")) hostPort.substringAfter('[').substringBefore(']') else hostPort.substringBefore(':')
        return host.takeIf { it.isNotEmpty() }
    }

    /**
     * [url] as it is safe to show: the characters that can change how text reads (bidirectional controls, zero-width characters, other
     * invisible formatting) are written out as `\uXXXX`, so that the real destination cannot be disguised.
     */
    fun displayable(url: String): String = buildString {
        for (c in url) {
            val type = Character.getType(c)
            if (type == Character.FORMAT.toInt() || type == Character.CONTROL.toInt() || type == Character.LINE_SEPARATOR.toInt() || type == Character.PARAGRAPH_SEPARATOR.toInt()) {
                append("\\u").append("%04X".format(c.code))
            } else append(c)
        }
    }

    /**
     * Whether the page of a message may load [url]. Defence in depth under the sanitiser: while remote content is blocked nothing may
     * leave the device; when it is allowed only `https` may. `data:` (the core's own inline pictures) and `about:blank` never leave the
     * device, so they pass in both cases.
     */
    fun requestAllowed(url: String?, remoteAllowed: Boolean): Boolean {
        val s = url?.let { schemeOf(it.trim()) } ?: return false
        if (s == "data" || s == "about") return true
        return remoteAllowed && s == "https"
    }
}
