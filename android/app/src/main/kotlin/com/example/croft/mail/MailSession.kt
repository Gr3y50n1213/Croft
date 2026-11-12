// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.design.CroftTokens
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
 * What the mail screens share for the life of the process: the repository (the real core; the sample mailbox is one of its accounts), the persisted scroll anchors, the clock and time zone (injected, so tests and screenshots see the same
 * times), the per-folder cache, the folder mapping, the Undo time and the events the screens raise. The activity creates one
 * ([MailRuntime.session]); tests build their own.
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
    /**
     * True for the empty stand-in session used only while the Rust core could not be opened (secure storage not ready, data from a
     * newer Croft): the app then shows the sign-in screens, which say so, and never this session's Inbox. The old Kotlin fake-data
     * Inbox is gone from the running app: "Try with sample mail" is the core's own sample account (`AccountInfo.is_demo`).
     */
    val coreUnavailable: Boolean = false,
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
        @Volatile private var unavailable: MailSession? = null

        /** The stand-in session while the core cannot be opened: no accounts, no rows ([coreUnavailable]). One per process. */
        fun unavailable(): MailSession = unavailable ?: synchronized(this) {
            unavailable ?: MailSession(
                FakeMessageRepository(emptyList(), System::currentTimeMillis, emptyList()),
                coreUnavailable = true,
            ).also { unavailable = it }
        }
    }
}
