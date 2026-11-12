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

    /**
     * What a tap on [url] does. A web link comes back in its **canonical** form ([canonicalWeb]): the one string that the dialog shows,
     * whose host is shown in bold, and that the browser is given. So the destination the person reads is the one the browser opens.
     */
    fun classify(url: String?): LinkAction {
        val text = url?.trim().orEmpty()
        if (text.isEmpty() || text.length > MAX_URL) return LinkAction.Ignore
        if (text.any { it.isISOControl() }) return LinkAction.Ignore
        return when (schemeOf(text)) {
            "http", "https" -> canonicalWeb(text)?.let { LinkAction.Web(it) } ?: LinkAction.Ignore
            "mailto" -> LinkAction.Mail(text)
            else -> LinkAction.Ignore
        }
    }

    // Characters a browser refuses in a host (WHATWG "forbidden host code point"), checked after percent-decoding.
    private const val FORBIDDEN_HOST = " #%/:<>?@[\\]^|"

    /**
     * A web link rewritten the way a browser reads it (WHATWG URL parsing for http and https), or null when it cannot be read safely:
     *  - `\` counts as `/` (browsers do this for http and https, so `https://evil.example\@bank.com` goes to evil.example);
     *  - extra slashes after the scheme are dropped;
     *  - a user name or password before the host (`https://bank.com@evil.example`) is removed, so it cannot pose as the host;
     *  - the host is percent-decoded, mapped and turned into ASCII (IDNA: look-alike letters show as `xn--`, full-width dots become dots),
     *    lower-cased, and must contain no character a browser refuses in a host;
     *  - a port must be digits.
     * The rest (path, query, fragment) is kept as written, apart from `\`.
     */
    fun canonicalWeb(url: String): String? {
        val s = schemeOf(url) ?: return null
        if (s != "http" && s != "https") return null
        // Exactly two slashes after the scheme (counting `\` as `/`, as browsers do); odder forms such as `https:///x` or
        // `https:x` are not made tappable at all.
        val slashed = url.substring(url.indexOf(':') + 1).replace('\\', '/')
        if (!slashed.startsWith("//") || slashed.startsWith("///")) return null
        val afterScheme = slashed.substring(2)
        val end = afterScheme.indexOfFirst { it == '/' || it == '?' || it == '#' }.let { if (it < 0) afterScheme.length else it }
        val authority = afterScheme.substring(0, end).substringAfterLast('@')
        val rest = afterScheme.substring(end)
        val rawHost: String
        val port: String?
        if (authority.startsWith("[")) {
            val close = authority.indexOf(']')
            if (close < 0) return null
            rawHost = authority.substring(0, close + 1)
            val after = authority.substring(close + 1)
            port = if (after.isEmpty()) null else if (after.startsWith(":")) after.substring(1) else return null
            if (!rawHost.drop(1).dropLast(1).all { it.isDigit() || it.lowercaseChar() in 'a'..'f' || it == ':' || it == '.' }) return null
        } else {
            rawHost = authority.substringBefore(':')
            port = if (authority.contains(':')) authority.substringAfter(':') else null
        }
        if (port != null && (port.length > 5 || !port.all { it in '0'..'9' })) return null
        val host = if (rawHost.startsWith("[")) rawHost.lowercase() else asciiHost(rawHost) ?: return null
        if (host.isEmpty() || host.trimEnd('.').isEmpty()) return null
        val portPart = if (port.isNullOrEmpty()) "" else ":$port"
        return "$s://$host$portPart$rest"
    }

    /** A host as a browser would use it: percent-decoded, IDNA-mapped to ASCII, lower case; null if it is not a usable host. */
    private fun asciiHost(raw: String): String? {
        val decoded = percentDecode(raw) ?: return null
        val ascii = try {
            java.net.IDN.toASCII(decoded, java.net.IDN.ALLOW_UNASSIGNED)
        } catch (_: IllegalArgumentException) {
            return null
        }.lowercase()
        if (ascii.isEmpty() || ascii.any { it.code < 0x21 || it.code > 0x7E || it in FORBIDDEN_HOST }) return null
        return ascii
    }

    /** `%XX` decoding as UTF-8; null for a broken escape or bytes that are not UTF-8. */
    private fun percentDecode(s: String): String? {
        if ('%' !in s) return s
        val out = java.io.ByteArrayOutputStream()
        var i = 0
        while (i < s.length) {
            val c = s[i]
            if (c == '%') {
                if (i + 2 >= s.length) return null
                val b = s.substring(i + 1, i + 3).toIntOrNull(16) ?: return null
                out.write(b)
                i += 3
            } else {
                out.write(c.toString().toByteArray(Charsets.UTF_8))
                i++
            }
        }
        val bytes = out.toByteArray()
        val decoder = Charsets.UTF_8.newDecoder()
        return try {
            decoder.decode(java.nio.ByteBuffer.wrap(bytes)).toString()
        } catch (_: java.nio.charset.CharacterCodingException) {
            null
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

    /** A link found in plain text: the characters [start] until [end] (exclusive) are the link, which goes to [url]. */
    data class LinkSpan(val start: Int, val end: Int, val url: String)

    private val inText = Regex("""(?i)\b(?:https?://|mailto:)[^\s<>"]+""")
    private const val TRAILING = ".,;:!?)]}'\u201D\u2019"

    /**
     * The web and `mailto:` links written out in plain [text], for the plain-text view. A link ends at white space; punctuation that
     * ends a sentence is not part of it. A link is only made tappable here: what it does is decided by [classify] and the person's answer.
     */
    fun linkSpans(text: String): List<LinkSpan> = inText.findAll(text).mapNotNull { m ->
        var end = m.range.last + 1
        while (end > m.range.first && text[end - 1] in TRAILING) end--
        val url = text.substring(m.range.first, end)
        if (classify(url) is LinkAction.Ignore) null else LinkSpan(m.range.first, end, url)
    }.toList()

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
    fun requestAllowed(url: String?, remoteAllowed: Boolean, method: String? = "GET", mainFrame: Boolean = false): Boolean {
        val s = url?.let { schemeOf(it.trim()) } ?: return false
        if (s == "data" || s == "about") return true
        // A message's page only ever fetches pictures: never a new page (navigation goes through the link dialog) and never a POST.
        if (mainFrame || (method != null && !method.equals("GET", ignoreCase = true))) return false
        return remoteAllowed && s == "https"
    }
}
