// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import android.content.Context
import com.example.croft.design.CroftTokens
import java.io.File
import java.time.ZoneId
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow

/** The rows a list showed last time in this process, so coming back to a folder shows them at once (no skeleton). */
class CachedList(val rows: List<MessageRowData>, val next: RowCursor?)

/** Something the mail screens tell the rest of the app (the screens that come later listen). */
sealed interface MailEvent {
    /** The person pressed Sign in on the Needs sign-in banner of [account]; the sign-in screen (T17) opens. */
    data class SignInRequested(val account: AccountId) : MailEvent
}

/**
 * What the mail screens share for the life of the process: the repository (the real core, or the fake sample mailbox while no
 * account exists), the persisted scroll anchors, the clock and time zone (injected, so tests and screenshots see the same
 * times), the per-folder cache, the folder mapping, the Undo time and the events the screens raise. The activity creates one
 * ([MailRuntime.session], or [shared] for the sample mailbox); tests build their own.
 */
class MailSession(
    val repository: MessageRepository,
    val anchors: AnchorStore = InMemoryAnchorStore(),
    val clock: () -> Long = System::currentTimeMillis,
    val zone: ZoneId = ZoneId.systemDefault(),
    /** What a swipe, a menu item or a TalkBack action does to a row: over the repository until T16 supplies the core's own. */
    val rowActions: RowActions = RepositoryRowActions(repository),
    /** What the person chose once for an account without an Archive, Trash or Spam folder, and the recent Move to folders. */
    val folderMapping: FolderMapping = InMemoryFolderMapping(),
    /** True for the app's fake-data sample mailbox (no account yet): the rows are made up in Kotlin. Not the core's sample account (`AccountInfo.is_demo`), which is a real core account. */
    val usesFakeData: Boolean = false,
) {
    private val cache = HashMap<String, CachedList>()

    /** The Undo time in ms (S49): how long the Undo snackbar of a move, delete or spam stays. The core is given the same number. */
    @Volatile var undoWindowMs: Long = CroftTokens.Swipe.UndoWindowSeconds.default * 1000L

    private val eventFlow = MutableSharedFlow<MailEvent>(extraBufferCapacity = 8)

    /** What the screens raised (the sign-in request of the banner). */
    val events: SharedFlow<MailEvent> get() = eventFlow

    /** Whether anything listens to [events] right now (the sign-in screen, once T17 has one). */
    val hasEventListener: Boolean get() = eventFlow.subscriptionCount.value > 0

    /** Raise [event] (a screen does). Never blocks; with no listener it is dropped. */
    fun raise(event: MailEvent) { eventFlow.tryEmit(event) }

    /** The last list shown for [scopeKey], or null on the first load of a folder in this process. */
    @Synchronized fun cached(scopeKey: String): CachedList? = cache[scopeKey]

    /** Remember what a list showed. */
    @Synchronized fun remember(scopeKey: String, rows: List<MessageRowData>, next: RowCursor?) { cache[scopeKey] = CachedList(rows, next) }

    companion object {
        /** The rows of the app's own demo mailbox: 5,000, as T14 asks. The real core's session replaces it once an account exists. */
        const val DEMO_ROWS = 5_000

        @Volatile private var shared: MailSession? = null

        /** The sample mailbox of this process: the fake repository with [DEMO_ROWS] rows, anchors in `noBackupFilesDir`. Shown until the first account exists (onboarding is T17). */
        fun shared(context: Context): MailSession = shared ?: synchronized(this) {
            shared ?: run {
                val now = System.currentTimeMillis()
                val repo = FakeMessageRepository(FakeData.rows(DEMO_ROWS, now), System::currentTimeMillis, FakeData.accounts(now))
                MailSession(repo, FileAnchorStore(File(context.applicationContext.noBackupFilesDir, "croft-list-anchors.json")), usesFakeData = true)
            }.also { shared = it }
        }
    }
}
