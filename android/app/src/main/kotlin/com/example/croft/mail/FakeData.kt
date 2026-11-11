// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

/**
 * Made-up mail for the fake repository: three accounts (all names and addresses are invented, on reserved example domains),
 * the usual folders per account, and any number of rows in every state a row can have. The same seed gives the same rows, so
 * screenshots and tests see the same list.
 */
object FakeData {
    /** The folder positions inside an account: its folder ids are `account * 100 + position`. */
    const val INBOX = 1L
    const val STARRED = 2L
    const val SNOOZED = 3L
    const val SENT = 4L
    const val DRAFTS = 5L
    const val OUTBOX = 6L
    const val ARCHIVE = 7L
    const val JUNK = 8L
    const val TRASH = 9L
    const val RECEIPTS = 10L
    const val WORK = 11L

    /** The id of folder [position] of [account]. */
    fun folderId(account: AccountId, position: Long): FolderId = account * 100 + position

    /** The three accounts: one syncing, one that will need a sign-in when a test says so, one up to date. */
    fun accounts(now: Long): List<AccountInfo> = listOf(
        AccountInfo(1, "Sam", "sam@example.org", AccountState.OK, now - 4 * MINUTE),
        AccountInfo(2, "Work", "sam@acme.test", AccountState.OK, now - 4 * MINUTE),
        AccountInfo(3, "Personal", "sam.rivera@example.net", AccountState.OK, now - 4 * MINUTE),
    )

    /** The folders every account has: special use and path. */
    val folderSpecs: List<Pair<Long, Pair<String, String>>> = listOf(
        INBOX to ("inbox" to "Inbox"),
        STARRED to ("starred" to "Starred"),
        SNOOZED to ("snoozed" to "Snoozed"),
        SENT to ("sent" to "Sent"),
        DRAFTS to ("drafts" to "Drafts"),
        OUTBOX to ("outbox" to "Outbox"),
        ARCHIVE to ("archive" to "Archive"),
        JUNK to ("junk" to "Spam"),
        TRASH to ("trash" to "Trash"),
        RECEIPTS to ("" to "Receipts"),
        WORK to ("" to "Work"),
    )

    private val people = listOf(
        "Sam Rivera" to "sam.rivera@example.org",
        "Hollis Printing" to "orders@hollis.example",
        "Mira Okafor" to "mira@example.net",
        "Fenwick Council" to "bins@fenwick.example",
        "Croft Dev List" to "list@croft.example",
        "Nora Lindqvist" to "nora@example.com",
        "Priya Shah" to "priya@acme.test",
        "Tomas Bell" to "tomas@example.org",
        "Elif Aydin" to "elif@example.net",
        "Joss Whitcombe" to "joss@example.com",
        "Ada Nwosu" to "ada@acme.test",
        "Library Services" to "library@town.example",
    )
    private val subjects = listOf(
        "Lunch on Friday?", "Your proof is ready", "Re: Server migration notes", "Bin collection changes",
        "[croft-dev] Weekly digest", "Photos from the weekend", "Invoice 4471 attached", "Meeting moved to Thursday",
        "Quarterly figures", "Book ready to collect", "Re: Re: Draft for review", "Parcel on its way",
        "Notes from the call", "Reminder: renewal", "A small favour", "Allotment rota", "Train times for Saturday",
        "Agenda for Monday", "Updated floor plan", "Thank you!", "Spare ticket", "Minutes attached",
    )
    private val snippets = listOf(
        "Are you free around noon? I was thinking of the place by the bridge.",
        "Open the link to approve the final proof before we print.",
        "Thanks, I will test the new DNS settings tonight and report back.",
        "From next week collection moves to Tuesday for the whole street.",
        "Three new issues, two merged pull requests and a release candidate.",
        "Here are the ones from the hike, the light was lovely on the ridge.",
        "Please find the invoice attached. Payment is due within thirty days.",
        "Can we move our meeting to Thursday afternoon? Mornings are full.",
        "I have put the numbers in the shared sheet; the totals are at the bottom.",
        "Your reserved copy is waiting at the front desk until the end of the week.",
    )

    /**
     * [count] rows, newest first by date with ids that grow with the date, spread over the accounts and folders (about 70
     * percent in an Inbox), with a share of unread, starred, attachment, encrypted, conversation and Sent rows. The six newest
     * Inbox rows are the ones of the wireframes.
     */
    fun rows(count: Int, now: Long, seed: Long = 1L, accounts: List<AccountInfo> = accounts(now)): List<MessageRowData> {
        val rng = Rng(seed)
        val out = ArrayList<MessageRowData>(count)
        var date = now
        // Newest first: walk back in time by irregular gaps.
        for (n in 0 until count) {
            val newestFirstIndex = n
            date -= if (n == 0) 2 * MINUTE else (3 + rng.next(22)) * MINUTE
            val id = (count - n).toLong()
            val sample = sampleRow(newestFirstIndex, id, date, accounts.first().id)
            if (sample != null && count >= SAMPLE_ROWS) { out += sample; continue }
            val account = accounts[rng.next(100).let { if (it < 45) 0 else if (it < 80) 1 else 2 }.coerceAtMost(accounts.size - 1)].id
            val bucket = rng.next(100)
            val (position, outgoing) = when {
                bucket < 70 -> INBOX to false
                bucket < 78 -> SENT to true
                bucket < 81 -> DRAFTS to true
                bucket < 89 -> ARCHIVE to false
                bucket < 92 -> JUNK to false
                bucket < 95 -> TRASH to false
                bucket < 98 -> RECEIPTS to false
                else -> WORK to false
            }
            val person = people[rng.next(people.size)]
            val seen = position != INBOX || rng.next(100) >= 22
            out += MessageRowData(
                id = id,
                account = account,
                folder = folderId(account, position),
                senderName = if (outgoing) "Sam" else person.first,
                senderAddress = if (outgoing) accounts.first { it.id == account }.email else person.second,
                subject = subjects[rng.next(subjects.size)],
                snippet = snippets[rng.next(snippets.size)],
                dateMs = date,
                seen = seen,
                flagged = rng.next(100) < 8,
                hasAttachments = rng.next(100) < 15,
                threadCount = if (rng.next(100) < 15) 2 + rng.next(7) else 1,
                encrypted = rng.next(100) < 5,
                recipients = if (outgoing) person.first else null,
            )
        }
        return out
    }

    private const val SAMPLE_ROWS = 6

    private fun sampleRow(n: Int, id: Long, date: Long, account: AccountId): MessageRowData? {
        fun row(name: String, address: String, subject: String, snippet: String, seen: Boolean, flagged: Boolean = false, clip: Boolean = false, thread: Int = 1) =
            MessageRowData(id, account, folderId(account, INBOX), name, address, subject, snippet, date, seen, flagged, clip, threadCount = thread)
        return when (n) {
            0 -> row("Sam Rivera", "sam.rivera@example.org", "Lunch on Friday?", snippets[0], seen = false)
            1 -> row("Hollis Printing", "orders@hollis.example", "Your proof is ready", snippets[1], seen = false, clip = true)
            2 -> row("Mira Okafor", "mira@example.net", "Re: Server migration notes", snippets[2], seen = true, flagged = true, thread = 3)
            3 -> row("Fenwick Council", "bins@fenwick.example", "Bin collection changes", snippets[3], seen = true)
            4 -> row("Croft Dev List", "list@croft.example", "[croft-dev] Weekly digest", snippets[4], seen = true)
            5 -> row("Nora Lindqvist", "nora@example.com", "Photos from the weekend", snippets[5], seen = true, clip = true)
            else -> null
        }
    }

    /** A small deterministic generator (xorshift): the same numbers on every machine. */
    private class Rng(seed: Long) {
        private var state = seed * 0x9E3779B97F4A7C15uL.toLong() + 0x1234567L
        fun next(bound: Int): Int {
            state = state xor (state shl 13)
            state = state xor (state ushr 7)
            state = state xor (state shl 17)
            return ((state ushr 1) % bound).toInt()
        }
    }

    internal const val MINUTE = 60_000L
}
