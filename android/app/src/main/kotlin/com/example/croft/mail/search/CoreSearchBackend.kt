// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.search

import com.example.croft.mail.MessageRowData
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.MessageRow as CoreMessageRow
import uniffi.croft_ffi.SearchFailure
import uniffi.croft_ffi.SearchHit as CoreHit
import uniffi.croft_ffi.TextRange

/**
 * [SearchBackend] over the core (FFI v10): `searchLocal` (the device's index, read off the main thread) and `searchServer` (each
 * account's own server). The core's UTF-16 ranges are Kotlin string indices, given here as inclusive [IntRange]s. Rows of the sample
 * mailbox carry "Not saved to server" as in the list.
 */
class CoreSearchBackend(private val core: Core, private val io: CoroutineDispatcher = Dispatchers.IO) : SearchBackend {
    override suspend fun local(account: Long?, query: String, limit: Int): LocalSearchResult = withContext(io) {
        val r = guarded { core.searchLocal(account, query, limit.coerceAtLeast(0).toUInt()) }
        val demo = demoAccounts()
        LocalSearchResult(r.hits.map { it.toApp(demo) }, r.bodiesPartial)
    }

    override suspend fun server(account: Long?, query: String): ServerSearchResult {
        val r = guarded { core.searchServer(account, query) }
        val demo = withContext(io) { demoAccounts() }
        return ServerSearchResult(
            hits = r.hits.map { it.toApp(demo) },
            unsupported = r.unsupported,
            offline = r.offline,
            signInNeeded = r.signInNeeded,
            failed = r.failed,
        )
    }

    private fun demoAccounts(): Set<Long> = try {
        core.listAccounts().filter { it.isDemo }.map { it.id }.toSet()
    } catch (e: Exception) {
        emptySet()
    }

    private inline fun <T> guarded(block: () -> T): T = try {
        block()
    } catch (e: SearchFailure.BadDate) {
        throw BadSearchDate()
    }

    private fun CoreHit.toApp(demo: Set<Long>) = SearchHitData(
        row = row.toApp(row.account in demo),
        subjectMatches = subjectRanges.toIntRanges(),
        snippet = snippet,
        snippetMatches = snippetRanges.toIntRanges(),
    )

    private fun List<TextRange>.toIntRanges() = map { it.start.toInt() until it.end.toInt() }.filterNot { it.isEmpty() }

    private fun CoreMessageRow.toApp(localOnly: Boolean) = MessageRowData(
        id = id, account = account, folder = folder, senderName = senderName, senderAddress = senderAddress,
        subject = subject, snippet = snippet, dateMs = dateMs, seen = seen, flagged = flagged,
        hasAttachments = hasAttachments, pending = pending, threadCount = threadCount.toInt().coerceAtLeast(1),
        recipients = recipients.joinToString(", ").ifBlank { null }, localOnly = localOnly,
    )
}
