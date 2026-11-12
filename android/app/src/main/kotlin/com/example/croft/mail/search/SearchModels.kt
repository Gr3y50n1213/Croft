// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.search

import com.example.croft.mail.MessageRowData

/** One search result: a list row and what matched in it (`[first, last]` char indices of the strings shown). */
data class SearchHitData(
    val row: MessageRowData,
    val subjectMatches: List<IntRange> = emptyList(),
    /** A piece of the downloaded body around a match, shown as the row's preview when present. */
    val snippet: String? = null,
    val snippetMatches: List<IntRange> = emptyList(),
)

/** What the device's index found. */
data class LocalSearchResult(
    val hits: List<SearchHitData>,
    /** Some bodies are not downloaded, so their text was not searched ("Searched headers and downloaded mail"). */
    val bodiesPartial: Boolean,
)

/** What the servers found, and what kept them from finding more (several may hold when all accounts are searched). */
data class ServerSearchResult(
    val hits: List<SearchHitData>,
    val unsupported: Boolean = false,
    val offline: Boolean = false,
    val signInNeeded: Boolean = false,
    val failed: Boolean = false,
)

/** The query has a date that is not a date (`before:` or `after:`). The only error a person can cause. */
class BadSearchDate : Exception("bad date in the search")

/** Search on the device and on the servers (the core, or a fake in tests). `account` null means all accounts. */
interface SearchBackend {
    /** Instant, offline too. Throws [BadSearchDate]. */
    suspend fun local(account: Long?, query: String, limit: Int): LocalSearchResult

    /** Each account's own server; cancelling stops the requests. Throws [BadSearchDate]. */
    suspend fun server(account: Long?, query: String): ServerSearchResult
}

/** The recent searches of S27, kept on the device only (never in a cloud). Newest first. */
interface SearchHistory {
    fun list(): List<String>

    /** Adds [query] at the top (once: an older copy moves up). */
    fun add(query: String)

    /** Removes [query]; returns where it was, for Undo ([restore]), or -1. */
    fun remove(query: String): Int

    /** Puts [query] back at [index] (Undo of [remove]). */
    fun restore(query: String, index: Int)

    fun clear()
}

/** A history in memory, capped at [max] entries. Also the base of the stored one. */
open class InMemorySearchHistory(initial: List<String> = emptyList(), private val max: Int = MAX) : SearchHistory {
    protected val items = ArrayList(initial.take(max))

    @Synchronized override fun list(): List<String> = items.toList()

    @Synchronized override fun add(query: String) {
        val q = query.trim()
        if (q.isEmpty()) return
        items.remove(q)
        items.add(0, q)
        while (items.size > max) items.removeAt(items.lastIndex)
        changed()
    }

    @Synchronized override fun remove(query: String): Int {
        val i = items.indexOf(query)
        if (i >= 0) {
            items.removeAt(i)
            changed()
        }
        return i
    }

    @Synchronized override fun restore(query: String, index: Int) {
        if (query in items) return
        items.add(index.coerceIn(0, items.size), query)
        while (items.size > max) items.removeAt(items.lastIndex)
        changed()
    }

    @Synchronized override fun clear() {
        items.clear()
        changed()
    }

    /** Called after every change (the stored history saves here). */
    protected open fun changed() {}

    companion object {
        const val MAX = 20
    }
}

/** The filter chips of S27 and S28: each is one operator of the query, added or removed as a whole word. */
enum class FilterChip(val token: String) {
    UNREAD("is:unread"),
    STARRED("is:starred"),
    ATTACHMENT("has:attachment"),
    ;

    /** Whether [query] has this chip's operator. */
    fun isOn(query: String): Boolean = query.split(Regex("\\s+")).any { it.equals(token, ignoreCase = true) }

    /** [query] with this chip's operator added or removed. */
    fun toggle(query: String): String =
        if (isOn(query)) query.split(Regex("\\s+")).filterNot { it.equals(token, ignoreCase = true) }.joinToString(" ").trim()
        else listOf(query.trim(), token).filter { it.isNotEmpty() }.joinToString(" ")
}
