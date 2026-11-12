// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.MessageRowData
import uniffi.croft_ffi.DraftSummary

/**
 * The Drafts folder shows the server's copy of a draft next to the device's own draft (core README, "Known gaps": both have the same
 * Message-ID and the app should hide the synced one). A list row does not carry its Message-ID yet, so a server row is taken for the copy
 * of a local draft when it is in the same account and has **the same subject and the same start of text**. That can only hide a row that
 * looks exactly like the local draft shown in its place, never one with other content; when a copy is missed both rows show. When
 * `MessageRow` carries the Message-ID this becomes an exact match.
 */
object LocalDraftMatch {
    /** Whether [row] (a message of a Drafts folder) is the server's copy of [draft]. */
    fun isCopyOf(row: MessageRowData, draft: DraftSummary): Boolean {
        if (row.account != draft.accountId) return false
        if (squash(row.subject) != squash(draft.subject)) return false
        val a = squash(row.snippet)
        val b = squash(draft.snippet)
        return a.startsWith(b) || b.startsWith(a)
    }

    /** [rows] without the server copies of [drafts]. */
    fun withoutCopies(rows: List<MessageRowData>, drafts: List<DraftSummary>): List<MessageRowData> =
        if (drafts.isEmpty()) rows else rows.filter { row -> drafts.none { isCopyOf(row, it) } }

    private fun squash(text: String): String = text.trim().replace(Regex("\\s+"), " ")
}
