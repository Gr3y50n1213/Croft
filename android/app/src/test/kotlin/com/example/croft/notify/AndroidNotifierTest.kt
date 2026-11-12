// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import com.example.croft.CroftRobolectricRunner
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class AndroidNotifierTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val manager = context.getSystemService(NotificationManager::class.java)
    private val shadow get() = shadowOf(manager)
    private val work = CheckAccount(1, "Work")
    private val home = CheckAccount(2, "Home")

    @get:Rule val tmp = TemporaryFolder()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After fun stop() = scope.cancel()

    private fun active(): List<Notification> = manager.activeNotifications.map { it.notification }
    private fun children(account: Long) = manager.activeNotifications.filter { it.tag == AndroidNotifier.mailTag(account) }
    private fun summary(account: Long) = manager.activeNotifications.singleOrNull { it.tag == AndroidNotifier.summaryTag(account) }?.notification

    private fun savedIntent(pi: android.app.PendingIntent): Intent = shadowOf(pi).savedIntent

    @Test
    fun eachAccountHasItsOwnChannelAndGroupWithASummary() {
        val n = AndroidNotifier(context)
        n.postNewMail(work, listOf(mail(1, 10, 5), mail(1, 11, 6)), NotifySettings())
        n.postNewMail(home, listOf(mail(2, 20, 7)), NotifySettings())
        assertEquals("Work", manager.getNotificationChannel(AndroidNotifier.channelId(1)).name)
        assertEquals("Home", manager.getNotificationChannel(AndroidNotifier.channelId(2)).name)
        assertEquals(2, children(1).size)
        assertTrue(children(1).all { it.notification.channelId == AndroidNotifier.channelId(1) && it.notification.group == AndroidNotifier.group(1) })
        val s = summary(1)!!
        assertTrue((s.flags and Notification.FLAG_GROUP_SUMMARY) != 0)
        assertEquals("2 new messages", s.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertEquals("1 new message", summary(2)!!.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
    }

    @Test
    fun aNotificationShowsSenderAndSubjectOnlyAndAPublicCountOnTheLockScreen() {
        AndroidNotifier(context).postNewMail(work, listOf(mail(1, 10, 5, subject = "Lunch tomorrow")), NotifySettings())
        val n = children(1).single().notification
        assertEquals("Ann Example", n.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals("Lunch tomorrow", n.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertNull("no body text, no big text style", n.extras.getCharSequence(Notification.EXTRA_BIG_TEXT))
        assertNull(n.extras.getCharSequence(Notification.EXTRA_SUMMARY_TEXT))
        assertEquals(Notification.CATEGORY_EMAIL, n.category)
        assertEquals(Notification.VISIBILITY_PRIVATE, n.visibility)
        val pub = assertNotNullAnd(n.publicVersion)
        assertEquals("1 new message", pub.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertFalse(pub.extras.toString().contains("Lunch"))
        assertFalse(pub.extras.toString().contains("Ann"))
    }

    @Test
    fun turningOffTheLockScreenOptionMakesItPublic() {
        AndroidNotifier(context).postNewMail(work, listOf(mail(1, 10, 5)), NotifySettings(hideOnLockScreen = false))
        val n = children(1).single().notification
        assertEquals(Notification.VISIBILITY_PUBLIC, n.visibility)
        assertNull(n.publicVersion)
    }

    @Test
    fun theTapOpensTheMessageThroughTheNonExportedAlias() {
        AndroidNotifier(context).postNewMail(work, listOf(mail(1, 10, 5)), NotifySettings())
        val tap = savedIntent(children(1).single().notification.contentIntent)
        assertEquals("croft://message/1/10/5", tap.dataString)
        assertEquals(AndroidNotifier.OPEN_ALIAS, tap.component!!.className)
        assertEquals(NotificationOpen.Message(1, 10, 5u), NotificationIntents.parse(tap, context))
    }

    @Test
    fun theChosenActionsAreTheButtonsWithTheirIntents() {
        val settings = NotifySettings(actions = listOf(NotifyAction.ARCHIVE, NotifyAction.DELETE, NotifyAction.REPLY))
        AndroidNotifier(context).postNewMail(work, listOf(mail(1, 10, 5)), settings)
        val actions = children(1).single().notification.actions
        assertEquals(listOf("Archive", "Delete", "Reply"), actions.map { it.title.toString() })
        val archive = savedIntent(actions[0].actionIntent)
        assertEquals(NotificationActionReceiver::class.java.name, archive.component!!.className)
        assertEquals(NotificationActionReceiver.ACTION, archive.action)
        assertEquals("croft-action://archive/1/10/5", archive.dataString)
        assertEquals("croft-action://delete/1/10/5", savedIntent(actions[1].actionIntent).dataString)
        val reply = savedIntent(actions[2].actionIntent)
        assertEquals("croft://reply/1/10/5", reply.dataString)
        assertEquals(NotificationOpen.Reply(1, 10, 5u), NotificationIntents.parse(reply, context))
        assertTrue(actions.all { shadowOf(it.actionIntent).isImmutable })
        // Archive and Delete change mail without opening Croft: a locked phone asks to unlock first. Reply opens Croft anyway.
        assertTrue(actions[0].isAuthenticationRequired && actions[1].isAuthenticationRequired)
        assertFalse(actions[2].isAuthenticationRequired)
    }

    @Test
    fun withNotificationsOffNothingIsPosted() {
        shadow.setNotificationsEnabled(false)
        val posted = AndroidNotifier(context).postNewMail(work, listOf(mail(1, 10, 5)), NotifySettings())
        assertTrue(posted.isEmpty())
        assertTrue(active().isEmpty())
        assertFalse(AndroidNotifier(context).postSignIn(work))
    }

    @Test
    fun withNotificationsOffTheRunnerPostsNothingAndTheMarkerDoesNotPileUp() = runBlocking {
        shadow.setNotificationsEnabled(false)
        val checker = FakeChecker(listOf(work), mutableMapOf(1L to CheckOutcome(listOf(mail(1, 10, 5)))))
        CheckRunner(checker, AndroidNotifier(context), NotifySettingsStore.onFile(File(tmp.root, "n.json"), scope)).run()
        assertTrue(active().isEmpty())
        assertEquals(mapOf(1L to listOf(5u)), checker.marked)
    }

    @Test
    fun theSignInNotificationNamesTheAccountAndCanBeCleared() {
        val n = AndroidNotifier(context)
        assertTrue(n.postSignIn(work))
        val s = manager.activeNotifications.single { it.tag == AndroidNotifier.signInTag(1) }.notification
        assertEquals(AndroidNotifier.STATUS_CHANNEL, s.channelId)
        assertTrue(s.extras.getCharSequence(Notification.EXTRA_TEXT).toString().startsWith("Work"))
        n.clearSignIn(1)
        assertTrue(active().isEmpty())
    }

    @Test
    fun theTestNotificationGoesThroughTheSamePathAndOpensTheInbox() {
        assertTrue(AndroidNotifier(context).postTest(work, NotifySettings()))
        val n = children(1).single().notification
        assertEquals("Croft", n.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals("croft://inbox", savedIntent(n.contentIntent).dataString)
        assertEquals(0, n.actions?.size ?: 0)
    }

    @Test
    fun removingAnAccountRemovesItsNotificationsAndChannel() {
        val n = AndroidNotifier(context)
        n.postNewMail(work, listOf(mail(1, 10, 5)), NotifySettings())
        n.postNewMail(home, listOf(mail(2, 20, 7)), NotifySettings())
        n.forgetAccount(1)
        assertTrue(children(1).isEmpty())
        assertNull(summary(1))
        assertNull(manager.getNotificationChannel(AndroidNotifier.channelId(1)))
        assertEquals(1, children(2).size)
    }

    private fun assertNotNullAnd(n: Notification?): Notification { assertNotNull(n); return n!! }
}
