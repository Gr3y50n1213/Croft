// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.unit.width
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.swipe
import androidx.compose.ui.test.center
import androidx.compose.ui.test.down
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.up
import androidx.compose.ui.geometry.Offset
import com.example.croft.haptics.HapticEvent
import com.example.croft.mail.AccountState
import com.example.croft.mail.FakeData
import com.example.croft.mail.ListRequest
import com.example.croft.mail.MessageScope
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.SwipeEnabled
import com.example.croft.ui.components.isInteractive
import com.example.croft.ui.components.show
import java.time.ZoneId
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The Inbox (S10), selection mode (S11), the drawer (S12), the account switcher (S13), pull to refresh and new mail (S17),
 * offline and sync problems (S18) and the empty states (S19), on the fake repository. Test names start with the screen ID.
 * The phone is 360 by 800 dp at 160 dpi, so 1 dp is 1 px.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class InboxScreenTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val inbox = MessageScope.UnifiedInbox

    private fun back() {
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
    }

    private fun exists(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
    private fun textExists(text: String, substring: Boolean = false) = rule.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()
    private fun waitForText(text: String, substring: Boolean = false) = rule.waitUntil(10_000) { textExists(text, substring) }
    private fun waitForNoText(text: String, substring: Boolean = false) = rule.waitUntil(10_000) { !textExists(text, substring) }
    private fun longPress(id: Long) { rule.rowCard(id).performTouchInput { longClick() }; rule.waitForIdle() }

    private fun open(f: InboxFixture = InboxFixture(300), level: AnimationLevel = AnimationLevel.FULL): InboxFixture {
        rule.showInbox(f, level)
        rule.waitForRow(f.topId)
        return f
    }

    // ================================================ S10 Inbox ================================================

    @Test
    fun s10_theInboxShowsTheWireframeRows_withDateHeadersAndTheChrome() {
        val f = open()
        val n = f.rows.toLong()
        rule.onNodeWithText("Search in mail").assertIsDisplayed()
        rule.onNodeWithText("All").assertIsDisplayed()
        rule.onNodeWithTag(InboxTags.FAB).assertIsDisplayed()
        rule.onNodeWithTag(InboxTags.BOTTOM_BAR).assertIsDisplayed()
        rule.onNodeWithText("Lunch on Friday?").assertIsDisplayed()
        rule.onNodeWithText("Your proof is ready").assertIsDisplayed()
        rule.onNodeWithTag(InboxTags.header("today")).assertIsDisplayed()
        assertEquals(n, f.topId)
        rule.onNodeWithText("Mail").assertIsDisplayed()
        rule.onNodeWithText("Calendar").assertIsNotEnabled()
        rule.onNodeWithText("Contacts").assertIsNotEnabled()
    }

    @Test
    fun s10_onlyAllIsInTheCategoryRowUntilTheSmartInboxPhase() {
        open()
        rule.onNodeWithTag(InboxTags.CHIPS).assertIsDisplayed()
        assertFalse(textExists("Primary"))
        assertFalse(textExists("Promotions"))
    }

    @Test
    fun s10_pagingAsksForFiftyFirst_thenAHundredWhenTheListIsFifteenRowsFromTheEnd() {
        val f = open(InboxFixture(400))
        assertEquals(listOf(ListRequest(inbox, null, 50)), f.repo.requests)
        val loaded = f.repo.snapshot(inbox).take(50)
        fun indexOfRow(i: Int) = buildListItems(loaded, NOW, ZoneId.of("UTC")).indexOfFirst { it is ListItem.Row && it.row.id == loaded[i].id }

        // The 20th row at the top: the last row on screen is about the 29th of 50, more than 15 rows from the end.
        rule.onNodeWithTag(InboxTags.LIST).performScrollToIndex(indexOfRow(20))
        rule.waitForIdle()
        assertEquals("not yet: more than 15 rows left", 1, f.repo.requests.size)

        // The 40th row at the top: the end of the 50 is on screen.
        rule.onNodeWithTag(InboxTags.LIST).performScrollToIndex(indexOfRow(40))
        rule.waitUntil(10_000) { f.repo.requests.size >= 2 }
        assertEquals(ListRequest(inbox, loaded.last().cursor, 100), f.repo.requests[1])
        rule.waitForIdle()
        assertEquals("one request only, not one per frame", 2, f.repo.requests.size)
    }

    @Test
    fun s10_aFiveThousandRowFakeRendersItsFirstFrameUnderAGenerousBound() {
        val f = InboxFixture(5_000)
        val start = System.nanoTime()
        rule.showInbox(f)
        rule.waitForRow(f.topId, timeoutMs = 60_000)
        val seconds = (System.nanoTime() - start) / 1e9
        // A smoke test, not a performance claim: the bound is generous on purpose (the JVM, a cold Robolectric).
        assertTrue("first frame took $seconds s", seconds < 60)
        assertTrue("only the rows on screen are composed, not the 50 loaded ones", rule.composedRows() < 30)
        assertEquals("the first page is 50 rows however many exist", listOf(ListRequest(inbox, null, 50)), f.repo.requests)
    }

    // ================================================ Skeleton and refresh ================================================

    @Test
    fun s10_skeletonRowsShowOnTheFirstLoadOfAFolder_andGoWhenTheRowsArrive() {
        val f = InboxFixture(300)
        f.repo.holdQueries()
        rule.showInbox(f)
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SKELETON).assertExists()
        assertTrue("a screenful of placeholders", rule.onAllNodes(hasTestTag(InboxTags.SKELETON_ROW)).fetchSemanticsNodes().size >= 8)
        rule.onNodeWithTag(InboxTags.LIST).assertDoesNotExist()
        f.repo.release()
        rule.waitForRow(f.topId)
        rule.onNodeWithTag(InboxTags.SKELETON).assertDoesNotExist()
    }

    @Test
    fun s10_aRefreshNeverShowsSkeletonRows_theRowsStayWhileTheQueryIsSlow() {
        val f = open()
        // The row on screen before the new mail: `f.topId` is read now, because after the injection it is the new, not yet listed, message.
        val shown = f.topId
        f.repo.holdQueries()
        f.repo.injectNewMail(1)
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SKELETON).assertDoesNotExist()
        rule.onNodeWithTag(InboxTags.LIST).assertExists()
        rule.onNodeWithTag(InboxTags.row(shown)).assertExists()
        f.repo.release()
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SKELETON).assertDoesNotExist()
    }

    @Test
    fun s10_comingBackToAFolderShowsItsRowsAtOnce_noSkeletonTheSecondTime() {
        val f = InboxFixture()
        var shown by mutableStateOf(true)
        rule.show(AnimationLevel.OFF, f.haptics) { if (shown) InboxScreen(f.session, onOpenSettings = {}) }
        rule.waitForRow(f.topId)
        rule.runOnUiThread { shown = false }
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SCREEN).assertDoesNotExist()
        // A slow query now would show a skeleton if the folder counted as new; it does not, it is cached.
        f.repo.holdQueries()
        rule.runOnUiThread { shown = true }
        rule.waitForRow(f.topId)
        rule.onNodeWithTag(InboxTags.SKELETON).assertDoesNotExist()
        f.repo.release()
    }

    // ================================================ S17 pull to refresh and new mail ================================================

    @Test
    fun s17_newMailBelowTheTopShowsThePill_andTheRowOnScreenDoesNotMove() {
        val f = open()
        val ids = f.inboxIds()
        rule.scrollToRow(ids[12])
        val before = rule.rowTop(ids[12])
        f.repo.injectNewMail(3)
        rule.waitUntil(10_000) { exists(InboxTags.NEW_PILL) }
        rule.waitForIdle()
        val after = rule.rowTop(ids[12])
        assertEquals("the first row on screen keeps its top offset", before.value, after.value, 0.5f)
        rule.onNodeWithText("3 new").assertExists()
        rule.onNodeWithTag(InboxTags.NEW_PILL).assertContentDescriptionEquals("3 new messages. Scroll to the top")
        assertEquals("the pill is a polite live region", LiveRegionMode.Polite, rule.onNodeWithTag(InboxTags.NEW_PILL).fetchSemanticsNode().config.getOrNull(SemanticsProperties.LiveRegion))
    }

    @Test
    fun s17_morePillCountsGrowAsMoreMailArrives_andTappingItGoesToTheTop() {
        val f = open()
        val ids = f.inboxIds()
        rule.scrollToRow(ids[10])
        f.repo.injectNewMail(2)
        rule.waitUntil(10_000) { exists(InboxTags.NEW_PILL) }
        f.repo.injectNewMail(1)
        rule.waitUntil(10_000) { textExists("3 new") }
        rule.onNodeWithTag(InboxTags.NEW_PILL).performClick()
        rule.waitUntil(10_000) { !exists(InboxTags.NEW_PILL) }
        rule.waitForIdle()
        val newest = f.inboxIds().first()
        rule.waitForRow(newest)
        rule.onNodeWithTag(InboxTags.row(newest)).assertIsDisplayed()
    }

    @Test
    fun s17_atTheVeryTopNewRowsAppearInPlace_withoutAPill() {
        val f = open()
        val fresh = f.repo.injectNewMail(2)
        rule.waitForRow(fresh.first())
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.row(fresh.first())).assertIsDisplayed()
        rule.onNodeWithTag(InboxTags.NEW_PILL).assertDoesNotExist()
    }

    @Test
    fun s17_pullingPastTheEightyDpThresholdRefreshes_andShortOfItDoesNot() {
        val f = open()
        f.clockMs += 60_000
        // 100 px of drag counts as 50 dp of pull: short of 80 dp.
        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { swipe(Offset(centerX, 40f), Offset(centerX, 140f), durationMillis = 300) }
        rule.waitForIdle()
        assertFalse("short of the threshold: no ThresholdArmed", HapticEvent.ThresholdArmed in f.haptics.events)
        assertTrue("and no sync", runBlocking { f.repo.listAccounts() }.all { it.lastSyncedMs != f.clockMs })

        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { swipe(Offset(centerX, 40f), Offset(centerX, 500f), durationMillis = 600) }
        rule.waitUntil(10_000) { runBlocking { f.repo.listAccounts() }.all { it.lastSyncedMs == f.clockMs } }
        assertTrue("armed at the threshold, with its haptic", HapticEvent.ThresholdArmed in f.haptics.events)
    }

    @Test
    fun s17_refreshNeverShowsSkeletonRows_andTheIndicatorStaysUntilTheSyncIsDone() {
        val f = open()
        f.repo.holdQueries()
        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { swipe(Offset(centerX, 40f), Offset(centerX, 500f), durationMillis = 600) }
        rule.waitUntil(10_000) { exists(InboxTags.PULL_INDICATOR) }
        rule.onNodeWithTag(InboxTags.SKELETON).assertDoesNotExist()
        rule.onNodeWithTag(InboxTags.row(f.topId)).assertExists()
        f.repo.release()
        rule.waitUntil(10_000) { !exists(InboxTags.PULL_INDICATOR) }
    }

    // ================================================ S11 selection mode ================================================

    @Test
    fun s11_longPressStartsSelection_withTheCount_theFabAndSwipeOff_andTheHapticGoesThroughHaptics() {
        val f = open()
        longPress(f.topId)
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertIsDisplayed()
        rule.onNodeWithText("1 selected").assertIsDisplayed()
        rule.rowCard(f.topId).assertIsSelected()
        assertEquals(HapticEvent.SelectionStart, f.haptics.events.first())
        rule.onNodeWithTag(InboxTags.FAB).assertDoesNotExist()
        rule.onNodeWithText("Mail").assertIsNotEnabled()
        assertEquals("swipe is off while selecting", false, rule.rowCard(f.topId).fetchSemanticsNode().config[SwipeEnabled])
        assertEquals(false, rule.rowCard(f.topId - 3).fetchSemanticsNode().config[SwipeEnabled])
    }

    @Test
    fun s11_swipeAndTheFabComeBackWhenSelectionEnds() {
        val f = open()
        assertEquals(true, rule.rowCard(f.topId).fetchSemanticsNode().config[SwipeEnabled])
        longPress(f.topId)
        rule.onNodeWithContentDescription("Exit selection").performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertDoesNotExist()
        rule.waitUntil(10_000) { exists(InboxTags.FAB) }
        assertEquals(true, rule.rowCard(f.topId).fetchSemanticsNode().config[SwipeEnabled])
        rule.onNodeWithText("Mail").assertIsEnabled()
    }

    @Test
    fun s11_selectInTheRowsAccessibilityActionsStartsSelection() {
        val f = open()
        // The avatar has no node of its own for screen readers; its route for them is the Select action of the row.
        val actions = rule.rowCard(f.topId).fetchSemanticsNode().config[SemanticsActions.CustomActions]
        rule.runOnUiThread { actions.first { it.label == "Select" }.action() }
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertIsDisplayed()
        rule.rowCard(f.topId).assertIsSelected()
    }

    @Test
    fun s11_theToolbarActionsFollowTheReadStateOfTheSelection() {
        val f = open()
        val unreadA = f.topId
        val unreadB = f.topId - 1
        val read = f.topId - 3
        longPress(read)
        rule.onNodeWithContentDescription("Mark as unread").assertExists()
        rule.onNodeWithContentDescription("Mark as read").assertDoesNotExist()
        rule.rowCard(unreadB).performClick()
        rule.waitForIdle()
        rule.onNodeWithText("2 selected").assertExists()
        rule.onNodeWithContentDescription("Mark as read").assertExists()
        rule.onNodeWithContentDescription("Mark as unread").assertDoesNotExist()
        rule.rowCard(unreadB).performClick()
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Mark as unread").assertExists()
        rule.onNodeWithContentDescription("Mark as read").assertDoesNotExist()
        rule.rowCard(unreadA).performClick()
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Mark as read").assertExists()
        assertTrue("every toggle was a selection tick", f.haptics.events.count { it == HapticEvent.SelectionTick } >= 3)
    }

    @Test
    fun s11_theInboxToolbarOffersArchiveAndDelete_andTheOverflowOffersStar() {
        val f = open()
        longPress(f.topId)
        rule.onNodeWithContentDescription("Archive").assertExists()
        rule.onNodeWithContentDescription("Delete").assertExists()
        rule.onNodeWithContentDescription("More actions").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Star").assertExists()
    }

    @Test
    fun s11_backExitsSelectionFirst_thenBehavesNormally() {
        val f = InboxFixture()
        var outerBacks = 0
        rule.show(AnimationLevel.OFF, f.haptics) {
            // The navigation's own handler is registered before the screen's, as in CroftNavHost.
            BackHandler { outerBacks++ }
            InboxScreen(f.session, onOpenSettings = {})
        }
        rule.waitForRow(f.topId)
        longPress(f.topId)
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertIsDisplayed()
        back()
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertDoesNotExist()
        assertEquals("the first Back only left selection mode", 0, outerBacks)
        back()
        assertEquals("the second Back went to the navigation", 1, outerBacks)
    }

    @Test
    fun s11_deselectingTheLastRowEndsSelection() {
        val f = open()
        longPress(f.topId)
        rule.rowCard(f.topId).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertDoesNotExist()
    }

    @Test
    fun s11_selectAllSelectsTheLoadedPage_andOffersSelectAllNInTheFolder() {
        val f = open()
        val total = f.inboxIds().size
        longPress(f.topId)
        rule.onNodeWithTag(InboxTags.SELECT_ALL).performClick()
        rule.waitForIdle()
        rule.onNodeWithText("50 selected").assertExists()
        rule.onNodeWithTag(InboxTags.SELECT_ALL_IN_FOLDER).assertExists()
        rule.onNodeWithText("Select all ${"%,d".format(total)} in this folder").assertExists()
        rule.onNodeWithTag(InboxTags.SELECT_ALL_IN_FOLDER).performClick()
        rule.waitForIdle()
        rule.onNodeWithText("All ${"%,d".format(total)} in this folder are selected").assertExists()
        rule.onNodeWithTag(InboxTags.SELECT_ALL_IN_FOLDER).assertDoesNotExist()
    }

    @Test
    fun s11_noSelectAllNOfferWhenEverythingIsLoaded() {
        val f = open(InboxFixture(20))
        longPress(f.topId)
        rule.onNodeWithTag(InboxTags.SELECT_ALL_IN_FOLDER).assertDoesNotExist()
    }

    @Test
    fun s11_archiveAppliesToEverythingSelected_withUndo_andFocusMovesToTheNextRow() {
        val f = open()
        val ids = f.inboxIds()
        longPress(ids[0])
        rule.rowCard(ids[1]).performClick()
        rule.onNodeWithContentDescription("Archive").performClick()
        waitForText("2 conversations archived")
        rule.waitUntil(10_000) { f.repo.snapshot(inbox).none { it.id == ids[0] || it.id == ids[1] } }
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertDoesNotExist()
        assertTrue(HapticEvent.Success in f.haptics.events)
        rule.waitForRow(ids[2])
        rule.waitForIdle()
        rule.rowCard(ids[2]).assertIsFocused()

        rule.onNodeWithText("Undo").performClick()
        rule.waitUntil(10_000) { f.repo.snapshot(inbox).any { it.id == ids[0] } }
        assertEquals("Undo put both back", ids.take(5), f.inboxIds().take(5))
    }

    @Test
    fun s11_deleteMovesToTrashWithUndo_andMarkReadAndStarHaveNoSnackbar() {
        val f = open()
        longPress(f.topId)
        rule.onNodeWithContentDescription("Mark as read").performClick()
        rule.waitUntil(10_000) { f.repo.snapshot(inbox).first { it.id == f.topId }.seen }
        assertFalse("no snackbar for read and unread", textExists("Undo"))
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertDoesNotExist()

        longPress(f.topId - 1)
        rule.onNodeWithContentDescription("Delete").performClick()
        waitForText("1 conversation deleted")
        assertTrue(f.repo.snapshot(inbox).none { it.id == f.topId - 1 })
        assertTrue(f.repo.snapshot(MessageScope.Folder(1, FakeData.folderId(1, FakeData.TRASH))).any { it.id == f.topId - 1 })
    }

    @Test
    fun s11_aBulkDeleteOfMoreThanFiftyAsksFirst_andCancelChangesNothing() {
        val f = open()
        val total = f.inboxIds().size
        longPress(f.topId)
        rule.onNodeWithTag(InboxTags.SELECT_ALL).performClick()
        rule.onNodeWithTag(InboxTags.SELECT_ALL_IN_FOLDER).performClick()
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Delete").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Delete ${"%,d".format(total)} conversations?".replace(",", "")).assertExists()
        rule.onNodeWithText("Cancel").performClick()
        rule.waitForIdle()
        assertEquals("nothing was deleted", total, f.inboxIds().size)
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertExists()

        rule.onNodeWithContentDescription("Delete").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Delete").performClick()
        rule.waitUntil(20_000) { f.inboxIds().isEmpty() }
        rule.waitUntil(10_000) { textExists("You're all caught up") }
    }

    @Test
    fun s11_aMissingArchiveFolderIsExplained_andNothingMoves() {
        val f = open()
        val row = f.repo.snapshot(inbox).first { it.account == 1L }
        f.repo.removeFolder(1, FakeData.ARCHIVE)
        scrollIfNeeded(row.id)
        longPress(row.id)
        rule.onNodeWithContentDescription("Archive").performClick()
        waitForText("Couldn't archive. This account has no Archive folder.")
        assertTrue(HapticEvent.Rejected in f.haptics.events)
        assertTrue(f.repo.snapshot(inbox).any { it.id == row.id })
    }

    private fun scrollIfNeeded(id: Long) {
        if (!exists(InboxTags.row(id))) rule.scrollToRow(id)
    }

    // ================================================ S12 drawer ================================================

    @Test
    fun s12_theDrawerListsMailboxesWithUnreadCounts_labels_andSettings() {
        val f = InboxFixture()
        rule.showInbox(f, startDrawerOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        rule.onNodeWithText("Croft Mail").assertIsDisplayed()
        // The drawer scrolls (nine mailboxes, labels and Settings do not all fit in 800 dp): the rest only has to be there.
        for (name in listOf("Inbox", "Starred", "Snoozed", "Sent", "Drafts", "Outbox", "Archive", "Spam", "Trash", "Labels", "Receipts", "Work", "Settings")) {
            rule.onNodeWithText(name).assertExists()
        }
        val unread = runBlocking { f.repo.listAccounts().flatMap { f.repo.listFolders(it.id) } }.filter { it.role == com.example.croft.mail.FolderRole.INBOX }.sumOf { it.unreadCount }
        assertTrue("some unread mail", unread > 0)
        rule.onNodeWithTag(InboxTags.drawerItem(FakeData.folderId(1, FakeData.INBOX))).assertTextContains(unread.toString(), substring = true)
    }

    @Test
    fun s12_settingsInTheDrawerLeavesForSettings_andTheDrawerClosesFirstOnBack() {
        val f = InboxFixture()
        var settings = 0
        rule.showInbox(f, onSettings = { settings++ }, startDrawerOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        back()
        rule.onNodeWithText("Croft Mail").assertIsNotDisplayed()
        assertEquals("Back closed the drawer and nothing else", 0, settings)
        rule.onNodeWithTag(InboxTags.MENU_BUTTON).performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Croft Mail").assertIsDisplayed()
        rule.onNodeWithTag("drawer-settings").performScrollTo().performClick()
        rule.waitForIdle()
        assertEquals(1, settings)
    }

    @Test
    fun s12_choosingAMailboxShowsItsRows_andTheDrawerCloses() {
        val f = InboxFixture()
        rule.showInbox(f, startDrawerOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        val sentFolder = FakeData.folderId(1, FakeData.SENT)
        val sent = f.repo.snapshot(MessageScope.Folder(1, sentFolder))
        assertTrue(sent.isNotEmpty())
        rule.onNodeWithTag(InboxTags.drawerItem(sentFolder)).performScrollTo().performClick()
        rule.waitForRow(sent.first().id)
        rule.waitForIdle()
        rule.onNodeWithText("Croft Mail").assertIsNotDisplayed()
        // Sent shows the recipients, not the sender, and says "To".
        assertTrue(rule.spoken(sent.first().id).contains("To ${sent.first().recipients}"))
    }

    // ================================================ S13 account switcher ================================================

    @Test
    fun s13_theSwitcherShowsEveryAccountWithAStatusThatSaysWhatToDo() {
        val f = InboxFixture()
        f.repo.setAccountStatus(1, AccountState.SYNCING)
        f.repo.setAccountStatus(2, AccountState.NEEDS_SIGN_IN)
        rule.showInbox(f, startSwitcherOpen = true)
        rule.waitForRow(f.topId)
        rule.waitUntil(10_000) { textExists("Needs sign-in") }
        rule.onNodeWithTag(InboxTags.SWITCHER).assertIsDisplayed()
        for (t in listOf("Accounts", "All accounts", "Sam", "sam@example.org", "Work", "sam@acme.test", "Personal", "sam.rivera@example.net", "Add another account", "Manage accounts")) {
            rule.onNode(hasText(t) and hasAnyAncestor(hasTestTag(InboxTags.SWITCHER))).assertExists()
        }
        rule.onNode(hasContentDescription("Account Work, Needs sign-in")).assertExists()
        rule.onNode(hasContentDescription("Account Sam, Syncing")).assertExists()
        assertTrue("the connected account says when it last synced", textExists("Synced", substring = true))
        rule.onNodeWithTag(InboxTags.switcherAccount(0)).assertIsSelected()
    }

    @Test
    fun s13_pickingAnAccountShowsItsInbox_andBackClosesTheSwitcherFirst() {
        val f = InboxFixture()
        rule.showInbox(f, startSwitcherOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        back()
        rule.onNodeWithTag(InboxTags.SWITCHER).assertDoesNotExist()
        rule.onNodeWithTag(InboxTags.ACCOUNT_BUTTON).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.switcherAccount(2)).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.SWITCHER).assertDoesNotExist()
        val work = f.repo.snapshot(MessageScope.Folder(2, FakeData.folderId(2, FakeData.INBOX)))
        rule.waitForRow(work.first().id)
        assertFalse("rows of the other accounts are gone", exists(InboxTags.row(f.topId)))
        rule.onNodeWithTag(InboxTags.ACCOUNT_BUTTON).assertContentDescriptionEquals("Account Work")
    }

    @Test
    fun s13_theAvatarSaysWhenAnAccountNeedsAttention() {
        val f = InboxFixture()
        f.repo.setAccountStatus(1, AccountState.NEEDS_SIGN_IN)
        open(f)
        rule.onNodeWithTag(InboxTags.ACCOUNT_BUTTON).assertContentDescriptionEquals("Account Sam, Needs sign-in")
    }

    // ================================================ S18 offline and sync problems ================================================

    @Test
    fun s18_airplaneMode_theListStaysUsable_theOfflineBannerShows_andActionsQueueWithTheClock() {
        val f = open()
        f.repo.setOffline(true)
        waitForText("You are offline. Changes wait and are sent later.")
        // Usable: the list scrolls, rows open selection, an action works.
        rule.scrollToRow(f.inboxIds()[10])
        rule.waitForRow(f.inboxIds()[10])
        rule.scrollToRow(f.topId)
        val actions = rule.rowCard(f.topId).fetchSemanticsNode().config[SemanticsActions.CustomActions]
        rule.runOnUiThread { actions.first { it.label == "Star" }.action() }
        rule.waitUntil(10_000) { rule.spoken(f.topId).contains("Waiting for the network") }
        assertTrue("the action applied locally at once", f.repo.snapshot(inbox).first { it.id == f.topId }.flagged)
        longPress(f.topId)
        rule.onNodeWithTag(InboxTags.SELECTION_BAR).assertIsDisplayed()
        rule.onNodeWithTag(InboxTags.ACCOUNT_BUTTON).assertDoesNotExist()
        rule.onNodeWithContentDescription("Exit selection").performClick()
        rule.onNodeWithTag(InboxTags.ACCOUNT_BUTTON).assertContentDescriptionEquals("Account Sam, Offline")

        f.repo.setOffline(false)
        waitForNoText("You are offline. Changes wait and are sent later.")
        rule.waitUntil(10_000) { !rule.spoken(f.topId).contains("Waiting for the network") }
    }

    @Test
    fun s18_onlyOneBannerShows_theMostUrgentOne() {
        val f = open()
        f.repo.setOffline(true)
        waitForText("You are offline. Changes wait and are sent later.")
        f.repo.setAccountStatus(2, AccountState.NEEDS_SIGN_IN)
        waitForText("Sign in again to Work")
        waitForNoText("You are offline. Changes wait and are sent later.")
        rule.onNodeWithText("Sign in").performClick()
        waitForText("This opens from the account screens", substring = true)
    }

    @Test
    fun s18_aServerProblemShowsItsBannerWithRetry_andRetryWorks() {
        val f = open()
        f.repo.setAccountStatus(1, AccountState.ERROR)
        waitForText("Couldn't reach the server for Sam. Croft will keep trying.")
        rule.onNodeWithText("Retry").performClick()
        waitForNoText("Couldn't reach the server", substring = true)
    }

    @Test
    fun s18_aDismissedBannerStaysAwayUntilTheProblemGoesAndComesBack() {
        val f = open()
        f.repo.setOffline(true)
        waitForText("You are offline", substring = true)
        rule.onNodeWithContentDescription("Dismiss").performClick()
        waitForNoText("You are offline", substring = true)
        f.repo.setAccountStatus(3, AccountState.NEEDS_SIGN_IN)
        waitForText("Sign in again to Personal")
        f.repo.setOffline(false)
        f.repo.setAccountStatus(3, AccountState.OK)
        waitForNoText("Sign in again", substring = true)
        f.repo.setOffline(true)
        waitForText("You are offline", substring = true)
    }

    @Test
    fun s18_aFailedRefreshSaysSoWithASnackbarAndTheRejectedHaptic() {
        val f = open()
        f.repo.setOffline(true)
        waitForText("You are offline", substring = true)
        rule.onNodeWithText("Mail").performClick() // at the top the active item refreshes
        waitForText("Couldn't refresh", substring = true)
        assertTrue(HapticEvent.Rejected in f.haptics.events)
    }

    // ================================================ S19 empty states ================================================

    private fun onlyInbox(n: Int = 30) = FakeData.rows(n, NOW).filter { it.folder % 100 == FakeData.INBOX }

    @Test
    fun s19_aCalmInboxZeroWithLastSyncedAndCompose() {
        val f = InboxFixture(0, data = emptyList())
        rule.showInbox(f)
        waitForText("You're all caught up")
        waitForText("Last synced", substring = true)
        rule.onNodeWithTag(InboxTags.EMPTY).assertIsDisplayed()
        rule.onAllNodes(hasText("Compose")).fetchSemanticsNodes().isNotEmpty().let { assertTrue(it) }
        rule.onNodeWithTag(InboxTags.LIST).assertDoesNotExist()
        rule.onNodeWithTag(InboxTags.SKELETON).assertDoesNotExist()
    }

    @Test
    fun s19_trashSpamAndOtherFoldersHaveTheirOwnShortCopy() {
        val f = InboxFixture(30, data = onlyInbox())
        rule.showInbox(f, startDrawerOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        for ((position, copy) in listOf(
            FakeData.TRASH to "Trash is empty",
            FakeData.JUNK to "No spam",
            FakeData.RECEIPTS to "This folder is empty",
        )) {
            rule.runOnUiThread { }
            rule.onNodeWithTag(InboxTags.MENU_BUTTON).performClick()
            rule.waitForIdle()
            rule.onNodeWithTag(InboxTags.drawerItem(FakeData.folderId(1, position))).performScrollTo().performClick()
            waitForText(copy)
            rule.onNodeWithTag(InboxTags.EMPTY).assertIsDisplayed()
        }
    }

    // ================================================ Accessibility ================================================

    @Test
    fun a11y_aRowIsReadStateFirst_thenTheContent_thePreviewLast() {
        val f = open()
        val unread = rule.spoken(f.topId)
        assertTrue(unread, unread.startsWith("Unread. From Sam Rivera. Lunch on Friday?. 2 minutes ago. "))
        assertTrue(unread, unread.endsWith("Are you free around noon? I was thinking of the place by the bridge."))

        val third = rule.spoken(f.topId - 2)
        val order = listOf("Read.", "From Mira Okafor.", "Re: Server migration notes.", " ago.", "3 messages in the conversation.", "Starred.", "Thanks, I will test")
        val positions = order.map { third.indexOf(it) }
        assertTrue(third, positions.all { it >= 0 })
        assertEquals("the order of the announcement: $third", positions.sorted(), positions)

        val withClip = rule.spoken(f.topId - 1)
        assertTrue(withClip, withClip.indexOf("Has attachment.") in (withClip.indexOf(" ago.") + 1) until withClip.indexOf("Open the link"))
    }

    @Test
    fun a11y_everyRowHasCustomActionsSoNoOneNeedsASwipeOrALongPress() {
        val f = open()
        val labels = rule.rowCard(f.topId).fetchSemanticsNode().config[SemanticsActions.CustomActions].map { it.label }
        // Select and Star, then the configured swipe actions (the Triage default: right Mark read, left Delete), then Reply.
        assertEquals(listOf("Select", "Star", "Mark as read", "Delete", "Reply"), labels)
        val read = rule.rowCard(f.topId - 3).fetchSemanticsNode().config[SemanticsActions.CustomActions].map { it.label }
        assertTrue("Mark as unread on a read row", "Mark as unread" in read && "Mark as read" !in read)

        // `f.topId` is the newest message of the Inbox *now*; once the first row is archived it names the next one, so keep the id.
        val first = f.topId
        val delete = rule.rowCard(first).fetchSemanticsNode().config[SemanticsActions.CustomActions].first { it.label == "Delete" }
        rule.runOnUiThread { delete.action() }
        rule.waitUntil(10_000) { f.repo.snapshot(inbox).none { it.id == first } }
        waitForText("1 conversation deleted")
    }

    @Test
    fun a11y_everyInteractiveNodeIsAtLeast48DpAndHasAName() {
        val f = open()
        val nodes = rule.onAllNodes(isInteractive).fetchSemanticsNodes()
        assertTrue(nodes.size >= 8)
        // Every offender is listed (with its tag and role), not only the first one, so a failure says what to fix.
        val offenders = nodes.mapNotNull { n ->
            val w = n.touchBoundsInRoot.width
            val h = n.touchBoundsInRoot.height
            val desc = n.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString()
            val text = n.config.getOrNull(SemanticsProperties.Text)?.joinToString { it.text }
            val tag = n.config.getOrNull(SemanticsProperties.TestTag)
            val role = n.config.getOrNull(SemanticsProperties.Role)
            val who = "tag=$tag role=$role desc=$desc text=$text size=${w}x$h px at ${n.touchBoundsInRoot}"
            when {
                w < 47.5f || h < 47.5f -> "touch target under 48 dp: $who"
                desc.isNullOrBlank() && text.isNullOrBlank() -> "interactive node without a name: $who"
                else -> null
            }
        }
        assertTrue("${offenders.size} of ${nodes.size} interactive nodes are wrong:\n" + offenders.joinToString("\n"), offenders.isEmpty())
        // The icon-only buttons carry their label.
        for (label in listOf("Open menu")) rule.onNodeWithContentDescription(label).assertExists()
        rule.onNodeWithTag(InboxTags.ACCOUNT_BUTTON).assertContentDescriptionEquals("Account Sam")
    }

    @Test
    fun a11y_selectionBarIconsAllHaveLabels_andTheCountIsAPoliteLiveRegion() {
        val f = open()
        longPress(f.topId)
        for (label in listOf("Exit selection", "Archive", "Delete", "Mark as read", "More actions")) rule.onNodeWithContentDescription(label).assertExists()
        val count = rule.onNodeWithText("1 selected").fetchSemanticsNode()
        assertEquals(LiveRegionMode.Polite, count.config.getOrNull(SemanticsProperties.LiveRegion))
    }

    // ================================================ Haptics ================================================

    @Test
    fun haptics_selectionAndActionsGoThroughTheHapticsInterface_inOrder() {
        val f = open()
        longPress(f.topId)
        rule.rowCard(f.topId - 1).performClick()
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Archive").performClick()
        waitForText("2 conversations archived")
        assertEquals(listOf(HapticEvent.SelectionStart, HapticEvent.SelectionTick, HapticEvent.Success), f.haptics.events)
    }

    // ================================================ FAB ================================================

    @Test
    fun s10_theFabCollapsesToItsIconOnScrollDown_andGrowsBackOnScrollUp() {
        val f = open()
        val expanded = rule.onNodeWithTag(InboxTags.FAB).getUnclippedBoundsInRoot().width
        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { swipeUp(durationMillis = 400) }
        rule.waitForIdle()
        rule.waitUntil(10_000) { rule.onNodeWithTag(InboxTags.FAB).getUnclippedBoundsInRoot().width < expanded }
        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { swipeDown(durationMillis = 400) }
        rule.waitForIdle()
        rule.waitUntil(10_000) { rule.onNodeWithTag(InboxTags.FAB).getUnclippedBoundsInRoot().width >= expanded }
        assertTrue(f.topId > 0)
    }

    @Test
    fun s10_tappingTheActiveBottomBarItemScrollsToTheTop() {
        val f = open()
        val ids = f.inboxIds()
        rule.scrollToRow(ids[15])
        rule.onNodeWithText("Mail").performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(InboxTags.row(f.topId))).fetchSemanticsNodes().isNotEmpty() }
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.row(f.topId)).assertIsDisplayed()
    }
}
