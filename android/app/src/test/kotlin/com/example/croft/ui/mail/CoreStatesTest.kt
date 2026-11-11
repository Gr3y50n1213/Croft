// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.down
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.width
import com.example.croft.haptics.HapticEvent
import com.example.croft.mail.MailEvent
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.show
import com.example.croft.ui.swipe.LocalBackGestureInset
import com.example.croft.ui.swipe.LocalSwipeSettings
import com.example.croft.swipe.SwipeSettings
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.croft_ffi.AccountStatus

/**
 * The Inbox over the app's real [com.example.croft.mail.CoreMessageRepository] with a scripted core (see [ScriptedCore]): the states
 * S16 and S18 as the real mapping produces them, and what the screen does with a failure notice and a sign-in request.
 * Robolectric, no emulator.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CoreStatesTest {
    @get:Rule
    val rule = createComposeRule()

    private fun show(f: CoreFixture) {
        rule.show(AnimationLevel.OFF, f.haptics) {
            CompositionLocalProvider(LocalSwipeSettings provides SwipeSettings.Default, LocalBackGestureInset provides 24.dp) {
                InboxScreen(f.session, onOpenSettings = {})
            }
        }
        rule.waitForRow(10, 20_000)
    }

    private fun waitForText(text: String, substring: Boolean = false) =
        rule.waitUntil(20_000) { rule.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty() }

    private fun swipeDelete(id: Long) {
        val width = rule.onNodeWithTag(InboxTags.row(id)).getUnclippedBoundsInRoot().width.value
        rule.rowCard(id).performTouchInput {
            down(center)
            moveBy(Offset(-10f, 0f))
            moveBy(Offset(-width * 0.5f, 0f))
            up()
        }
    }

    @Test
    fun s16_aSwipeThroughTheRealRepositoryRemovesTheRowAndOffersUndo_undoBringsItBack() {
        val f = CoreFixture()
        show(f)
        swipeDelete(10)
        waitForText("1 conversation deleted")
        assertTrue("Undo is on the snackbar", rule.onAllNodes(hasText("Undo")).fetchSemanticsNodes().isNotEmpty())
        rule.waitUntil(20_000) { rule.onAllNodes(hasText("Lunch on Friday?")).fetchSemanticsNodes().isEmpty() }
        rule.onNodeWithText("Undo").performClick()
        rule.waitForRow(10, 20_000)
    }

    @Test
    fun s18_aRefusedChangePutsTheRowBackWithTheRedEdge_aRetrySnackbar_andTheRejectedHaptic() {
        val f = CoreFixture()
        show(f)
        swipeDelete(10)
        waitForText("1 conversation deleted")
        // The server refuses the move for good (operation 1 is the delete the swipe just made).
        f.core.reject(1uL, account = 1)
        rule.waitForRow(10, 20_000)
        waitForText("Couldn't finish that change", substring = true)
        assertTrue("Retry is offered", rule.onAllNodes(hasText("Retry")).fetchSemanticsNodes().isNotEmpty())
        assertTrue("the Rejected haptic played", HapticEvent.Rejected in f.haptics.events)
    }

    @Test
    fun s18_theOfflineBannerAndThePendingClockComeFromTheCoresStatusAndRows() {
        val f = CoreFixture()
        show(f)
        // The pending row (a change that waits) says so to a screen reader.
        rule.waitUntil(20_000) { rule.spoken(9).contains("Waiting for the network") }
        f.core.setStatus(1, AccountStatus.OFFLINE)
        waitForText("You are offline. Changes wait and are sent later.")
    }

    @Test
    fun s18_needsSignInShowsItsBanner_andTheButtonRaisesTheEventForTheSignInScreen() {
        val f = CoreFixture(statusOf = { if (it == 2L) AccountStatus.NEEDS_SIGN_IN else AccountStatus.OK })
        val raised = CompletableDeferred<MailEvent>()
        // The sign-in screen (T17) will listen like this.
        val listener = CoroutineScope(Dispatchers.Default).launch { raised.complete(f.session.events.first()) }
        show(f)
        waitForText("Sign in again to Work")
        rule.waitUntil(20_000) { f.session.hasEventListener }
        rule.onNodeWithText("Sign in").performClick()
        assertEquals(MailEvent.SignInRequested(2), runBlocking { withTimeout(20_000) { raised.await() } })
        listener.cancel()
        assertFalse("with a listener the placeholder note is not shown", rule.onAllNodes(hasText("This opens from the account screens", substring = true)).fetchSemanticsNodes().isNotEmpty())
    }
}
