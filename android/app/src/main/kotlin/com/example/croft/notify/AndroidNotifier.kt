// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.app.NotificationChannelCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.example.croft.MainActivity
import com.example.croft.R

/**
 * S32 basic notifications (T20), built on the device from what the core returned: **sender and subject only**, never a word of the body.
 * - One channel per account (`croft-mail-<id>`, named after the account). Sound and vibration belong to the channel, so the person sets
 *   them in system settings; Croft does not override them. A "Account problems" channel carries the one "Sign in again" notification.
 * - Each account's notifications form a group with a summary ("N new messages").
 * - Lock screen: private by default, with a public version that says only "N new messages" ([NotifySettings.hideOnLockScreen]).
 * - Up to three buttons ([NotifySettings.actions]): Archive, Delete and Mark read go to [NotificationActionReceiver] (not exported);
 *   Reply opens compose. A tap opens the message with the Inbox under it. Both open through the non-exported alias
 *   `OpenFromNotification`, so no other app can send Croft to a message.
 */
class AndroidNotifier(context: Context) : Notifier {
    private val context = context.applicationContext
    private val manager = NotificationManagerCompat.from(this.context)

    override fun canPost(): Boolean = manager.areNotificationsEnabled()

    override fun postNewMail(account: CheckAccount, mail: List<NewMail>, settings: NotifySettings): List<UInt> {
        if (!canPost()) return emptyList()
        ensureChannel(account)
        val posted = ArrayList<UInt>()
        // Oldest first, so the newest ends on top; only the newest few are shown one by one, the summary counts them all.
        for (m in mail.takeLast(MAX_SHOWN)) {
            try {
                manager.notify(mailTag(account.id), notificationId(m.messageId), message(account, m, settings))
                posted += m.uid
            } catch (e: SecurityException) {
                // The permission was taken away meanwhile: the rest is offered again.
                return posted
            }
        }
        // The older ones that were not shown one by one still count as notified (they are in the summary's number).
        mail.dropLast(MAX_SHOWN).forEach { posted += it.uid }
        refreshSummary(account, settings, extra = mail.size - minOf(mail.size, MAX_SHOWN))
        return posted
    }

    /** "Send test" (S07, S46): a real notification through the same path, with made-up content that opens the Inbox. */
    fun postTest(account: CheckAccount, settings: NotifySettings): Boolean {
        val test = NewMail(account.id, TEST_MESSAGE_ID, 0u, context.getString(R.string.notify_test_sender), null, context.getString(R.string.notify_test_subject), System.currentTimeMillis())
        return postNewMail(account, listOf(test), settings).isNotEmpty()
    }

    override fun postSignIn(account: CheckAccount): Boolean {
        if (!canPost()) return false
        ensureStatusChannel()
        val n = NotificationCompat.Builder(context, STATUS_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_croft)
            .setContentTitle(context.getString(R.string.notify_signin_title))
            .setContentText(context.getString(R.string.notify_signin_text, account.label))
            .setCategory(NotificationCompat.CATEGORY_ERROR)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(
                NotificationCompat.Builder(context, STATUS_CHANNEL)
                    .setSmallIcon(R.drawable.ic_stat_croft)
                    .setContentTitle(context.getString(R.string.notify_signin_title))
                    .build(),
            )
            .setContentIntent(openIntent(Uri.parse("croft://inbox")))
            .setAutoCancel(true)
            .build()
        return try {
            manager.notify(signInTag(account.id), 0, n)
            true
        } catch (e: SecurityException) {
            false
        }
    }

    override fun clearSignIn(account: Long) = manager.cancel(signInTag(account), 0)

    /** T36: "Back in your inbox": how many snoozed messages came back. Nothing of the messages themselves. */
    fun postBackFromSnooze(count: Int): Boolean {
        if (!canPost() || count <= 0) return false
        ensureStatusChannel()
        val text = context.resources.getQuantityString(R.plurals.notify_snooze_back, count, count)
        val n = NotificationCompat.Builder(context, STATUS_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_croft)
            .setContentTitle(context.getString(R.string.mail_unsnoozed))
            .setContentText(text)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setContentIntent(openIntent(Uri.parse("croft://inbox")))
            .setAutoCancel(true)
            .build()
        return try {
            manager.notify(SNOOZE_TAG, 0, n)
            true
        } catch (e: SecurityException) {
            false
        }
    }

    /**
     * T39b: an event reminder: the title and when it starts, on its own channel. A tap opens S37 for that occurrence. Its id comes from
     * the event, the occurrence and the time, so the same reminder posted twice is one notification.
     */
    fun postReminder(r: com.example.croft.mail.calendar.Reminder): Boolean {
        if (!canPost()) return false
        ensureReminderChannel()
        val time = java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(r.startMs))
        val n = NotificationCompat.Builder(context, REMINDER_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_croft)
            .setContentTitle(r.title.ifBlank { context.getString(R.string.calendar_untitled) })
            .setContentText(context.getString(R.string.notify_reminder_text, time))
            .setCategory(NotificationCompat.CATEGORY_EVENT)
            .setWhen(r.startMs)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setContentIntent(openIntent(Uri.parse("croft://event/${r.account}/${r.eventId}/${r.keyMs}")))
            .setAutoCancel(true)
            .build()
        return try {
            manager.notify(REMINDER_TAG, java.util.Objects.hash(r.account, r.eventId, r.keyMs, r.atMs), n)
            true
        } catch (e: SecurityException) {
            false
        }
    }

    /** Removes one message's notification and updates (or removes) the account's summary. */
    fun cancelMessage(account: Long, messageId: Long) {
        manager.cancel(mailTag(account), notificationId(messageId))
        val left = activeInGroup(account)
        if (left == 0) manager.cancel(summaryTag(account), 0)
    }

    /** Removes everything of [account] (it was removed): its notifications and its channel. */
    fun forgetAccount(account: Long) {
        system().activeNotifications.filter { it.tag in setOf(mailTag(account), summaryTag(account), signInTag(account), doneTag(account)) }
            .forEach { manager.cancel(it.tag, it.id) }
        manager.deleteNotificationChannel(channelId(account))
    }

    /** Removes every notification of Croft (Erase all data). Channels are deleted too. */
    fun forgetAll() {
        manager.cancelAll()
        manager.notificationChannelsCompat.filter { it.id.startsWith(MAIL_CHANNEL_PREFIX) }.forEach { manager.deleteNotificationChannel(it.id) }
    }

    private fun message(account: CheckAccount, m: NewMail, settings: NotifySettings): android.app.Notification {
        val title = m.sender.ifBlank { context.getString(R.string.notify_unknown_sender) }
        val subject = m.subject.ifBlank { context.getString(R.string.notify_no_subject) }
        val tap = if (m.messageId == TEST_MESSAGE_ID) Uri.parse("croft://inbox") else Uri.parse("croft://message/${m.account}/${m.messageId}/${m.uid}")
        val b = NotificationCompat.Builder(context, channelId(account.id))
            .setSmallIcon(R.drawable.ic_stat_croft)
            .setContentTitle(title)
            .setContentText(subject)
            .setSubText(account.label)
            .setWhen(m.dateMs)
            .setShowWhen(true)
            .setCategory(NotificationCompat.CATEGORY_EMAIL)
            .setGroup(group(account.id))
            .setOnlyAlertOnce(true)
            .setAutoCancel(true)
            .setContentIntent(openIntent(tap))
        lockScreen(b, account, settings, count = 1)
        if (m.messageId != TEST_MESSAGE_ID) {
            for (action in settings.actions.take(NotifySettings.MAX_ACTIONS)) {
                // Archive, Delete and Mark read change mail without opening Croft: on a locked phone they ask to unlock first (Android 12
                // and later), even when the person shows the whole notification on the lock screen. Reply opens Croft, which unlocks anyway.
                b.addAction(
                    NotificationCompat.Action.Builder(0, context.getString(actionLabel(action)), actionIntent(action, m))
                        .setAuthenticationRequired(action != NotifyAction.REPLY)
                        .build(),
                )
            }
        }
        return b.build()
    }

    private fun refreshSummary(account: CheckAccount, settings: NotifySettings, extra: Int) {
        val count = activeInGroup(account.id) + extra
        if (count <= 0) return
        val text = context.resources.getQuantityString(R.plurals.notify_new_count, count, count)
        val b = NotificationCompat.Builder(context, channelId(account.id))
            .setSmallIcon(R.drawable.ic_stat_croft)
            .setContentTitle(account.label)
            .setContentText(text)
            .setCategory(NotificationCompat.CATEGORY_EMAIL)
            .setGroup(group(account.id))
            .setGroupSummary(true)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setNumber(count)
            .setAutoCancel(true)
            .setContentIntent(openIntent(Uri.parse("croft://inbox")))
        lockScreen(b, account, settings, count)
        try { manager.notify(summaryTag(account.id), 0, b.build()) } catch (e: SecurityException) { /* nothing shows */ }
    }

    private fun lockScreen(b: NotificationCompat.Builder, account: CheckAccount, settings: NotifySettings, count: Int) {
        if (!settings.hideOnLockScreen) {
            b.setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            return
        }
        b.setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
        b.setPublicVersion(
            NotificationCompat.Builder(context, channelId(account.id))
                .setSmallIcon(R.drawable.ic_stat_croft)
                .setContentTitle(context.resources.getQuantityString(R.plurals.notify_new_count, count, count))
                .setCategory(NotificationCompat.CATEGORY_EMAIL)
                .build(),
        )
    }

    private fun activeInGroup(account: Long): Int =
        system().activeNotifications.count { it.tag == mailTag(account) && (it.notification.flags and android.app.Notification.FLAG_GROUP_SUMMARY) == 0 }

    private fun system(): NotificationManager = context.getSystemService(NotificationManager::class.java)

    private fun ensureChannel(account: CheckAccount) {
        manager.createNotificationChannel(
            NotificationChannelCompat.Builder(channelId(account.id), NotificationManagerCompat.IMPORTANCE_DEFAULT)
                .setName(account.label)
                .setDescription(context.getString(R.string.notify_channel_mail_description))
                .build(),
        )
    }

    private fun ensureReminderChannel() {
        manager.createNotificationChannel(
            NotificationChannelCompat.Builder(REMINDER_CHANNEL, NotificationManagerCompat.IMPORTANCE_HIGH)
                .setName(context.getString(R.string.notify_channel_reminders))
                .build(),
        )
    }

    private fun ensureStatusChannel() {
        manager.createNotificationChannel(
            NotificationChannelCompat.Builder(STATUS_CHANNEL, NotificationManagerCompat.IMPORTANCE_DEFAULT)
                .setName(context.getString(R.string.notify_channel_status))
                .build(),
        )
    }

    private fun openIntent(uri: Uri): PendingIntent {
        val intent = Intent(Intent.ACTION_VIEW, uri)
            .setClassName(context.packageName, OPEN_ALIAS)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        return PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    private fun actionIntent(action: NotifyAction, m: NewMail): PendingIntent {
        if (action == NotifyAction.REPLY) return openIntent(Uri.parse("croft://reply/${m.account}/${m.messageId}/${m.uid}"))
        val intent = Intent(context, NotificationActionReceiver::class.java)
            .setAction(NotificationActionReceiver.ACTION)
            .setData(Uri.parse("croft-action://${action.key}/${m.account}/${m.messageId}/${m.uid}"))
        return PendingIntent.getBroadcast(context, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    companion object {
        const val MAIL_CHANNEL_PREFIX = "croft-mail-"
        const val STATUS_CHANNEL = "croft-status"
        const val SNOOZE_TAG = "croft-snooze-back"
        const val REMINDER_CHANNEL = "croft-reminders"
        const val REMINDER_TAG = "croft-reminder"

        /** The most notifications of one account shown one by one per check; the summary counts the rest. */
        const val MAX_SHOWN = 8

        /** The made-up message of "Send test" (no row has this id). */
        const val TEST_MESSAGE_ID = -1L

        /** The non-exported activity-alias the notifications open (see the manifest). */
        val OPEN_ALIAS: String = MainActivity::class.java.name.substringBeforeLast('.') + ".OpenFromNotification"

        fun channelId(account: Long) = "$MAIL_CHANNEL_PREFIX$account"
        fun group(account: Long) = "croft-group-$account"
        fun mailTag(account: Long) = "croft-mail-$account"
        fun summaryTag(account: Long) = "croft-summary-$account"
        fun signInTag(account: Long) = "croft-signin-$account"
        fun doneTag(account: Long) = "croft-done-$account"
        fun notificationId(messageId: Long): Int = (messageId xor (messageId ushr 32)).toInt()

        fun actionLabel(action: NotifyAction): Int = when (action) {
            NotifyAction.ARCHIVE -> R.string.notify_action_archive
            NotifyAction.DELETE -> R.string.notify_action_delete
            NotifyAction.MARK_READ -> R.string.notify_action_read
            NotifyAction.REPLY -> R.string.notify_action_reply
        }
    }
}
