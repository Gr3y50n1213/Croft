// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

/** The shortcut chips of S02 (Add account). */
enum class ProviderShortcut(val domain: String?) {
    GMAIL("gmail.com"),
    OUTLOOK("outlook.com"),
    ICLOUD("icloud.com"),

    /** Any other provider: no domain is filled in. */
    OTHER(null),
}

/**
 * The checks S02 makes on the address before it lets Continue through. They are deliberately simple and only stop what can
 * never work (no `@`, no dot in the domain, a space): the core's own parser (`EmailAddress::parse`) has the last word and the
 * screen shows its `InvalidAddress` too. Plain Kotlin, no Android types, so the JVM tests run it.
 */
object EmailInput {
    /** True when [text] looks like `name@domain.tld`. Surrounding spaces are ignored. */
    fun isValid(text: String): Boolean {
        val t = text.trim()
        if (t.isEmpty() || t.any { it.isWhitespace() || it.isISOControl() }) return false
        val at = t.indexOf('@')
        if (at <= 0 || at != t.lastIndexOf('@')) return false
        val domain = t.substring(at + 1)
        val labels = domain.split('.')
        if (labels.size < 2) return false
        if (labels.any { it.isEmpty() || it.startsWith('-') || it.endsWith('-') }) return false
        return labels.last().length >= 2
    }

    /** The part after the `@`, lower-cased, or null when there is none. */
    fun domainOf(text: String): String? {
        val t = text.trim()
        val at = t.lastIndexOf('@')
        return if (at in 0 until t.length - 1) t.substring(at + 1).lowercase() else null
    }

    /** The part before the `@` (the whole text when there is none). */
    fun localPartOf(text: String): String {
        val t = text.trim()
        val at = t.indexOf('@')
        return if (at >= 0) t.substring(0, at) else t
    }

    /**
     * What a shortcut chip does to the text: Gmail, Outlook and iCloud set the domain (keeping what was typed before the `@`);
     * Other leaves the text alone. Never deletes the part the person typed.
     */
    fun applyShortcut(current: String, shortcut: ProviderShortcut): String {
        val domain = shortcut.domain ?: return current
        return localPartOf(current) + "@" + domain
    }
}
