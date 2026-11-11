// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import android.content.Context
import java.io.File
import java.time.ZoneId

/** The rows a list showed last time in this process, so coming back to a folder shows them at once (no skeleton). */
class CachedList(val rows: List<MessageRowData>, val next: RowCursor?)

/**
 * What the mail screens share for the life of the process: the repository (the fake until T16), the persisted scroll anchors,
 * the clock and time zone (injected, so tests and screenshots see the same times), and the per-folder cache. The activity
 * creates one ([shared]); tests build their own.
 */
class MailSession(
    val repository: MessageRepository,
    val anchors: AnchorStore = InMemoryAnchorStore(),
    val clock: () -> Long = System::currentTimeMillis,
    val zone: ZoneId = ZoneId.systemDefault(),
) {
    private val cache = HashMap<String, CachedList>()

    /** The last list shown for [scopeKey], or null on the first load of a folder in this process. */
    @Synchronized fun cached(scopeKey: String): CachedList? = cache[scopeKey]

    /** Remember what a list showed. */
    @Synchronized fun remember(scopeKey: String, rows: List<MessageRowData>, next: RowCursor?) { cache[scopeKey] = CachedList(rows, next) }

    companion object {
        /** The rows of the app's own demo mailbox: 5,000, as T14 asks. T16 replaces the whole session with the core's. */
        const val DEMO_ROWS = 5_000

        @Volatile private var shared: MailSession? = null

        /** The one session of this process: the fake repository with [DEMO_ROWS] rows, anchors in `noBackupFilesDir`. */
        fun shared(context: Context): MailSession = shared ?: synchronized(this) {
            shared ?: run {
                val now = System.currentTimeMillis()
                val repo = FakeMessageRepository(FakeData.rows(DEMO_ROWS, now), System::currentTimeMillis, FakeData.accounts(now))
                MailSession(repo, FileAnchorStore(File(context.applicationContext.noBackupFilesDir, "croft-list-anchors.json")))
            }.also { shared = it }
        }
    }
}
