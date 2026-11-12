// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.AccountId
import com.example.croft.mail.ChangeBatch
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import uniffi.croft_ffi.OutboxItem
import uniffi.croft_ffi.OutboxItemState

/** One line of the Outbox (S26): the core's item and the account it belongs to. */
data class OutboxRow(val account: AccountId, val item: OutboxItem) {
    val id: Long get() = item.id.toLong()

    // Subject and recipients are not printed (CLAUDE.md, security rules).
    override fun toString(): String = "OutboxRow(account=$account, id=$id, state=${item.state})"
}

/** A question before an Outbox action runs. */
sealed interface OutboxPrompt {
    /** "This message may already have been sent": Retry anyway or Cancel. The only way to `retry_outbox_item(..., confirmed = true)`. */
    data class ConfirmRetry(val row: OutboxRow) : OutboxPrompt

    /** Discard asks first. */
    data class ConfirmDiscard(val row: OutboxRow) : OutboxPrompt
}

/** What the Outbox screen draws. */
data class OutboxState(
    val loaded: Boolean = false,
    val rows: List<OutboxRow> = emptyList(),
    val prompt: OutboxPrompt? = null,
    /** An action was refused: the screen says so once. */
    val problem: ComposeProblem? = null,
)

/**
 * The logic of S26: Queued, Sending and Failed messages of every account, and their actions.
 *
 * - **Refresh.** [run] loads the lists and then re-reads those of the accounts a change notice names (`ChangeBatch.accounts`), or all
 *   of them when the notice says "everything", exactly as the Inbox follows the core.
 * - **Retry** on a message that [OutboxItem.mayHaveBeenSent] never calls the core until the person has answered "This message may already
 *   have been sent" with Retry anyway ([confirmRetry]); any other Failed message is retried at once with `confirmed = false`.
 * - **Edit** takes the message back to its draft ([edit] returns the compose start); refused while it is being sent.
 * - **Discard** asks first ([confirmDiscard]).
 */
class OutboxController(
    private val backend: ComposeBackend,
    private val accountIds: suspend () -> List<AccountId>,
    private val changes: suspend (after: Long) -> ChangeBatch,
    private val scope: CoroutineScope,
) {
    private val mutable = MutableStateFlow(OutboxState())

    /** What the screen draws. */
    val state: StateFlow<OutboxState> = mutable.asStateFlow()

    /** Loads, then follows the core's changes until cancelled. */
    suspend fun run() {
        refresh()
        var after = 0L
        while (true) {
            val batch = changes(after)
            after = batch.seq
            if (batch.everything || batch.accounts.isNotEmpty()) refresh(if (batch.everything) null else batch.accounts.toSet())
        }
    }

    /** Re-reads the Outbox of [only] (all accounts when null). */
    suspend fun refresh(only: Set<AccountId>? = null) {
        val accounts = try { accountIds() } catch (e: CancellationException) { throw e } catch (e: Exception) { return }
        val kept = mutable.value.rows.filter { r -> only != null && r.account !in only && r.account in accounts }
        val fresh = ArrayList<OutboxRow>()
        for (account in accounts) {
            if (only != null && account !in only) continue
            try {
                backend.listOutbox(account).forEach { fresh += OutboxRow(account, it) }
            } catch (e: ComposeFailure) {
                // That account has none to show right now.
            }
        }
        val rows = (kept + fresh).sortedWith(compareBy({ it.item.sendAtMs }, { it.id }))
        mutable.update { it.copy(loaded = true, rows = rows, prompt = it.prompt?.takeIf { p -> stillThere(p, rows) }) }
    }

    private fun stillThere(prompt: OutboxPrompt, rows: List<OutboxRow>): Boolean {
        val row = when (prompt) { is OutboxPrompt.ConfirmRetry -> prompt.row; is OutboxPrompt.ConfirmDiscard -> prompt.row }
        return rows.any { it.account == row.account && it.id == row.id }
    }

    /** Retry was tapped on a Failed row. */
    suspend fun retry(row: OutboxRow) {
        if (row.item.state != OutboxItemState.FAILED) return
        if (row.item.mayHaveBeenSent) {
            mutable.update { it.copy(prompt = OutboxPrompt.ConfirmRetry(row)) }
            return
        }
        runRetry(row, confirmed = false)
    }

    /** "Retry anyway": the only call with `confirmed = true`. */
    suspend fun confirmRetry() {
        val prompt = mutable.value.prompt as? OutboxPrompt.ConfirmRetry ?: return
        mutable.update { it.copy(prompt = null) }
        runRetry(prompt.row, confirmed = true)
    }

    private suspend fun runRetry(row: OutboxRow, confirmed: Boolean) {
        try {
            backend.retryOutboxItem(row.account, row.id, confirmed)
        } catch (e: ComposeFailure) {
            if (e.problem == ComposeProblem.NEEDS_CONFIRMATION) mutable.update { it.copy(prompt = OutboxPrompt.ConfirmRetry(row)) }
            else mutable.update { it.copy(problem = e.problem) }
        }
        refresh(setOf(row.account))
    }

    /** Edit: the message leaves the Outbox and compose opens on its draft. Null (and [OutboxState.problem]) when it cannot, for example while it is being sent. */
    suspend fun edit(row: OutboxRow): ComposeStart.Draft? {
        val start = try {
            ComposeStart.Draft(row.account, backend.editOutboxItem(row.account, row.id))
        } catch (e: ComposeFailure) {
            mutable.update { it.copy(problem = e.problem) }
            null
        }
        refresh(setOf(row.account))
        return start
    }

    /** Discard was tapped: asks first. */
    fun discard(row: OutboxRow) {
        if (row.item.state == OutboxItemState.SENDING) return
        mutable.update { it.copy(prompt = OutboxPrompt.ConfirmDiscard(row)) }
    }

    suspend fun confirmDiscard() {
        val prompt = mutable.value.prompt as? OutboxPrompt.ConfirmDiscard ?: return
        mutable.update { it.copy(prompt = null) }
        try {
            backend.discardOutboxItem(prompt.row.account, prompt.row.id)
        } catch (e: ComposeFailure) {
            mutable.update { it.copy(problem = e.problem) }
        }
        refresh(setOf(prompt.row.account))
    }

    fun cancelPrompt() { mutable.update { it.copy(prompt = null) } }

    fun problemShown() { mutable.update { it.copy(problem = null) } }

    /** The reason of [row] as the core's translation key (`outbox.reason.too_large`), or null. */
    fun reasonKey(row: OutboxRow): String? = row.item.reason?.let { uniffi.croft_ffi.outboxReasonKey(it) }
}
