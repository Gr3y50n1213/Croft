// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.app.Notification
import android.app.NotificationManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.test.core.app.ApplicationProvider
import com.example.croft.CroftRobolectricRunner
import com.example.croft.MainActivity
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** The action layer the buttons call: records the calls; Archive and Delete return made-up operation ids. */
class FakeActions(private val fail: Boolean = false) : NotifyActionsBackend {
    val calls = ArrayList<String>()
    override suspend fun undoWindowMs() = 5_000L
    override suspend fun archive(messageId: Long): List<Long> { calls += "archive:$messageId"; if (fail) error("no Archive folder"); return listOf(41) }
    override suspend fun trash(messageId: Long): List<Long> { calls += "trash:$messageId"; return listOf(42, 43) }
    override suspend fun markRead(messageId: Long) { calls += "read:$messageId" }
    override suspend fun undo(ops: List<Long>) { calls += "undo:${ops.joinToString(",")}" }
}

@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class NotifyActionHandlerTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val manager = context.getSystemService(NotificationManager::class.java)
    private val work = CheckAccount(1, "Work")

    private fun posted(actions: List<NotifyAction>) = AndroidNotifier(context).postNewMail(work, listOf(mail(1, 10, 5), mail(1, 11, 6)), NotifySettings(actions = actions))

    private fun button(messageId: Long, index: Int): Intent {
        val n = manager.activeNotifications.single { it.tag == AndroidNotifier.mailTag(1) && it.id == AndroidNotifier.notificationId(messageId) }.notification
        return shadowOf(n.actions[index].actionIntent).savedIntent
    }

    private fun mailIds() = manager.activeNotifications.filter { it.tag == AndroidNotifier.mailTag(1) }.map { it.id }.toSet()
    private fun done() = manager.activeNotifications.singleOrNull { it.tag == AndroidNotifier.doneTag(1) }?.notification

    @Test
    fun archiveCallsTheActionLayerAndReplacesTheNotificationWithAnUndoForTheUndoWindow() = runBlocking {
        posted(listOf(NotifyAction.ARCHIVE))
        val actions = FakeActions()
        NotifyActionHandler(context, actions).handle(button(10, 0))
        assertEquals(listOf("archive:10"), actions.calls)
        assertEquals(setOf(AndroidNotifier.notificationId(11)), mailIds())
        val d = done()!!
        assertEquals("Archived", d.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals(5_000L, d.timeoutAfter)
        assertEquals("Undo", d.actions.single().title.toString())

        NotifyActionHandler(context, actions).handle(shadowOf(d.actions.single().actionIntent).savedIntent)
        assertEquals(listOf("archive:10", "undo:41"), actions.calls)
        assertNull(done())
    }

    @Test
    fun deleteMovesToTrashWithUndo() = runBlocking {
        posted(listOf(NotifyAction.DELETE))
        val actions = FakeActions()
        NotifyActionHandler(context, actions).handle(button(11, 0))
        assertEquals(listOf("trash:11"), actions.calls)
        assertEquals("Moved to Trash", done()!!.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        NotifyActionHandler(context, actions).handle(shadowOf(done()!!.actions.single().actionIntent).savedIntent)
        assertEquals("undo:42,43", actions.calls.last())
    }

    @Test
    fun markReadRemovesTheNotificationAndTheSummaryGoesWithTheLastOne() = runBlocking {
        posted(listOf(NotifyAction.MARK_READ))
        val actions = FakeActions()
        NotifyActionHandler(context, actions).handle(button(10, 0))
        NotifyActionHandler(context, actions).handle(button(11, 0))
        assertEquals(listOf("read:10", "read:11"), actions.calls)
        assertTrue(manager.activeNotifications.isEmpty())
    }

    @Test
    fun aFailedActionLeavesTheNotificationAsItWas() = runBlocking {
        posted(listOf(NotifyAction.ARCHIVE))
        NotifyActionHandler(context, FakeActions(fail = true)).handle(button(10, 0))
        assertEquals(setOf(AndroidNotifier.notificationId(10), AndroidNotifier.notificationId(11)), mailIds())
        assertNull(done())
    }

    @Test
    fun anIntentThatIsNotOursDoesNothing() = runBlocking {
        val actions = FakeActions()
        NotifyActionHandler(context, actions).handle(Intent("other").setData(Uri.parse("croft-action://archive/1/10")))
        NotifyActionHandler(context, actions).handle(Intent(NotificationActionReceiver.ACTION).setData(Uri.parse("https://archive/1/10")))
        NotifyActionHandler(context, actions).handle(Intent(NotificationActionReceiver.ACTION).setData(Uri.parse("croft-action://archive/x/10")))
        assertTrue(actions.calls.isEmpty())
    }

    @Test
    fun onlyTheNonExportedAliasCanOpenAMessage() {
        val viaAlias = Intent(Intent.ACTION_VIEW, Uri.parse("croft://message/1/10")).setClassName(context.packageName, AndroidNotifier.OPEN_ALIAS)
        assertEquals(NotificationOpen.Message(1, 10), NotificationIntents.parse(viaAlias, context))
        // Another app can start MainActivity (it is exported for mailto: and shares), but not send it to a message.
        val direct = Intent(Intent.ACTION_VIEW, Uri.parse("croft://message/1/10")).setComponent(ComponentName(context, MainActivity::class.java))
        assertNull(NotificationIntents.parse(direct, context))
        val otherPackage = Intent(Intent.ACTION_VIEW, Uri.parse("croft://message/1/10")).setClassName("com.evil", AndroidNotifier.OPEN_ALIAS)
        assertNull(NotificationIntents.parse(otherPackage, context))
        val inbox = Intent(Intent.ACTION_VIEW, Uri.parse("croft://inbox")).setClassName(context.packageName, AndroidNotifier.OPEN_ALIAS)
        assertEquals(NotificationOpen.Inbox, NotificationIntents.parse(inbox, context))
        val junk = Intent(Intent.ACTION_VIEW, Uri.parse("croft://message/x/-3")).setClassName(context.packageName, AndroidNotifier.OPEN_ALIAS)
        assertEquals(NotificationOpen.Inbox, NotificationIntents.parse(junk, context))
    }
}
