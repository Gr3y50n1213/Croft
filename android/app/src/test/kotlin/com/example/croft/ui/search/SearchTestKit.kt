// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.search

import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.search.BadSearchDate
import com.example.croft.mail.search.LocalSearchResult
import com.example.croft.mail.search.SearchBackend
import com.example.croft.mail.search.SearchHitData
import com.example.croft.mail.search.ServerSearchResult
import java.util.concurrent.CopyOnWriteArrayList

/** Fictional rows for the search screens. */
fun searchRow(id: Long, sender: String, subject: String, snippet: String = "", account: Long = 1) = MessageRowData(
    id = id, account = account, folder = 1, senderName = sender, senderAddress = "${sender.lowercase().replace(' ', '.')}@example.org",
    subject = subject, snippet = snippet, dateMs = 1_794_400_000_000 - id * 3_600_000, seen = id % 2 == 0L, flagged = id == 2L,
    hasAttachments = id == 3L, pending = false, threadCount = 1, recipients = null,
)

val searchHits = listOf(
    SearchHitData(searchRow(1, "Mira Okafor", "Lunch on Friday?"), subjectMatches = listOf(0..4)),
    SearchHitData(
        searchRow(2, "Dan Reyes", "Team notes"),
        snippet = "…we could do lunch after the review…",
        snippetMatches = listOf(13..17),
    ),
    SearchHitData(searchRow(3, "Sam Rivera", "Lunch menu (attached)"), subjectMatches = listOf(0..4)),
)

val searchAccounts = listOf(
    AccountInfo(1, "Work", "sam@example.org", AccountState.OK),
    AccountInfo(2, "Home", "sam@example.net", AccountState.OK),
)

/** A search backend for the screens: fixed hits, a chosen server answer, and every call recorded. */
class FakeSearchBackend(
    var local: List<SearchHitData> = searchHits,
    var server: ServerSearchResult = ServerSearchResult(emptyList()),
    var bodiesPartial: Boolean = false,
) : SearchBackend {
    val calls = CopyOnWriteArrayList<String>()

    override suspend fun local(account: Long?, query: String, limit: Int): LocalSearchResult {
        calls += "local:$account:$query"
        if ("after:bad" in query) throw BadSearchDate()
        return LocalSearchResult(if (query.contains("nothing")) emptyList() else local, bodiesPartial)
    }

    override suspend fun server(account: Long?, query: String): ServerSearchResult {
        calls += "server:$account:$query"
        return server
    }
}
