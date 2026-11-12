// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import java.io.ByteArrayOutputStream

/**
 * `mailto:` links (RFC 6068) read into [ComposeFill]: `mailto:a@x.org,b@x.org?cc=..&bcc=..&subject=..&body=..`. Percent-escapes are
 * decoded as UTF-8; `+` stays a plus (it is not a space in a mailto). Header-like fields beyond these four are ignored, and **`attach`
 * is never honoured**: a link must not be able to attach a file from the phone. Line breaks are removed from the subject (no
 * header injection). Lengths are bounded.
 */
object Mailto {
    /** Cuts, in characters: a subject and a text from outside are bounded (the person is told when something was cut). */
    const val MAX_FIELD = 4_000
    const val MAX_BODY = 20_000
    /** The most recipients one link or share may name; more are ignored. */
    const val MAX_RECIPIENTS = 100

    /** The fill for [uri], or null when it is not a `mailto:` link. */
    fun parse(uri: String): ComposeFill? {
        val text = uri.trim()
        if (!text.startsWith("mailto:", ignoreCase = true)) return null
        val rest = text.substring("mailto:".length)
        val query = rest.substringAfter('?', "")
        val path = rest.substringBefore('?').substringBefore('#')
        var to = splitList(decode(path))
        var cc = emptyList<String>()
        var bcc = emptyList<String>()
        var subject = ""
        var body = ""
        var truncated = false
        for (pair in query.substringBefore('#').split('&')) {
            if (pair.isEmpty()) continue
            val key = decode(pair.substringBefore('=')).lowercase()
            val value = decode(pair.substringAfter('=', ""))
            when (key) {
                "to" -> to = to + splitList(value)
                "cc" -> cc = cc + splitList(value)
                "bcc" -> bcc = bcc + splitList(value)
                "subject" -> { truncated = truncated || value.length > MAX_FIELD; subject = value.replace("\r", " ").replace("\n", " ").take(MAX_FIELD) }
                "body" -> { truncated = truncated || value.length > MAX_BODY; body = value.replace("\r\n", "\n").take(MAX_BODY) }
                else -> Unit // including "attach" and "in-reply-to": never from a link
            }
        }
        return ComposeFill(to = to, cc = cc, bcc = bcc, subject = subject, body = body, truncated = truncated)
    }

    private fun splitList(text: String): List<String> = text.split(',', ';').map { it.trim() }.filter { it.isNotEmpty() }.take(MAX_RECIPIENTS)

    /** Percent-decoding as UTF-8; a bad escape stays as it is. */
    internal fun decode(text: String): String {
        if (!text.contains('%')) return text
        val out = ByteArrayOutputStream()
        var i = 0
        while (i < text.length) {
            val c = text[i]
            if (c == '%' && i + 2 < text.length) {
                val hi = Character.digit(text[i + 1], 16)
                val lo = Character.digit(text[i + 2], 16)
                if (hi >= 0 && lo >= 0) { out.write(hi * 16 + lo); i += 3; continue }
            }
            out.write(c.toString().toByteArray(Charsets.UTF_8))
            i++
        }
        return out.toString(Charsets.UTF_8.name())
    }
}
