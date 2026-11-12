// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.example.croft.R
import com.example.croft.mail.MailRuntime
import com.example.croft.mail.MailSettingsStore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** What a notification button does in the core: the same calls as the Inbox's actions (T16). A fake in tests. */
interface NotifyActionsBackend {
    /** The Undo window the person chose (S49), in milliseconds; the core waits this long before it sends a move. */
    suspend fun undoWindowMs(): Long

    suspend fun archive(messageId: Long): List<Long>

    suspend fun trash(messageId: Long): List<Long>

    suspend fun markRead(messageId: Long)

    suspend fun undo(ops: List<Long>)
}

/** [NotifyActionsBackend] over the process's mail session (opened if the process was cold). */
class SessionActionsBackend(private val context: Context) : NotifyActionsBackend {
    private suspend fun repository() = MailRuntime.session(context).repository

    override suspend fun undoWindowMs(): Long {
        val ms = MailSettingsStore.shared(context).settings.first().undoWindowMs
        // A cold process: the core does not know the person's Undo time yet.
        repository().setUndoWindowMs(ms)
        return ms
    }

    override suspend fun archive(messageId: Long): List<Long> = repository().archive(listOf(messageId))

    override suspend fun trash(messageId: Long): List<Long> = repository().trash(listOf(messageId))

    override suspend fun markRead(messageId: Long) = repository().markSeen(listOf(messageId), true)

    override suspend fun undo(ops: List<Long>) { ops.forEach { repository().undo(it) } }
}

/**
 * What a tap on Archive, Delete, Mark read or Undo does (S32). Archive and Delete honour the Undo window like a swipe: the message's
 * notification is replaced by a quiet "Archived" (or "Moved to Trash") one with **Undo**, which goes away by itself when the window ends,
 * and the core sends the move only then. Mark read removes the notification. A failure (no Archive folder, the core cannot open) leaves the
 * notification as it was, so nothing looks done that is not.
 */
class NotifyActionHandler(private val context: Context, private val backend: NotifyActionsBackend) {
    private val notifier = AndroidNotifier(context)
    private val manager = NotificationManagerCompat.from(context.applicationContext)

    suspend fun handle(intent: Intent) {
        if (intent.action != NotificationActionReceiver.ACTION) return
        val parts = intent.data?.takeIf { it.scheme == "croft-action" } ?: return
        val segments = parts.pathSegments
        val account = segments.getOrNull(0)?.toLongOrNull() ?: return
        val message = segments.getOrNull(1)?.toLongOrNull() ?: return
        try {
            when (parts.host) {
                NotifyAction.ARCHIVE.key -> moved(account, message, backend.archive(message), R.string.notify_done_archived)
                NotifyAction.DELETE.key -> moved(account, message, backend.trash(message), R.string.notify_done_deleted)
                NotifyAction.MARK_READ.key -> { backend.markRead(message); notifier.cancelMessage(account, message) }
                UNDO -> {
                    val ops = parts.getQueryParameter("ops")?.split(',')?.mapNotNull { it.toLongOrNull() }.orEmpty()
                    backend.undo(ops)
                    manager.cancel(AndroidNotifier.doneTag(account), AndroidNotifier.notificationId(message))
                }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Not done: the notification stays, and the person can open the message instead. No text is kept (it could name a folder).
        }
    }

    private suspend fun moved(account: Long, message: Long, ops: List<Long>, done: Int) {
        val window = backend.undoWindowMs()
        notifier.cancelMessage(account, message)
        if (ops.isEmpty() || !manager.areNotificationsEnabled()) return
        val undo = Intent(context, NotificationActionReceiver::class.java)
            .setAction(NotificationActionReceiver.ACTION)
            .setData(Uri.parse("croft-action://$UNDO/$account/$message?ops=${ops.joinToString(",")}"))
        val n = NotificationCompat.Builder(context, AndroidNotifier.channelId(account))
            .setSmallIcon(R.drawable.ic_stat_croft)
            .setContentTitle(context.getString(done))
            .setSilent(true)
            .setTimeoutAfter(window)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .addAction(0, context.getString(R.string.notify_action_undo), PendingIntent.getBroadcast(context, 0, undo, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            .build()
        try { manager.notify(AndroidNotifier.doneTag(account), AndroidNotifier.notificationId(message), n) } catch (e: SecurityException) { /* nothing shows */ }
    }

    companion object {
        const val UNDO = "undo"
    }
}

/**
 * The notification buttons' receiver (S32). **Not exported**: only Croft's own immutable PendingIntents reach it. The work runs in
 * [goAsync] on the IO dispatcher; the core's calls are the same as the Inbox's.
 */
class NotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pending = goAsync()
        scope.launch {
            try {
                NotifyActionHandler(context.applicationContext, SessionActionsBackend(context.applicationContext)).handle(intent)
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION = "com.example.croft.notify.ACTION"
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    }
}
