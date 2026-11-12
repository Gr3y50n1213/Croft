// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.printToString
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import com.example.croft.CroftRobolectricRunner
import com.example.croft.RustInfo
import com.example.croft.mail.FakeData
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MailSettings
import com.example.croft.mail.MailSettingsStore
import com.example.croft.mail.accounts.AccountColour
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.AccountPrefsStore
import com.example.croft.secrets.AesGcmSecretBox
import com.example.croft.secrets.MemoryKeySource
import com.example.croft.secrets.SecretKind
import com.example.croft.secrets.SecretStore
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeJson
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.mail.MoveTags
import com.example.croft.ui.nav.CroftApp
import com.example.croft.ui.nav.Route
import com.example.croft.ui.nav.SettingsTags
import com.example.croft.ui.onboarding.FakeBackend
import com.example.croft.ui.onboarding.FakeFirstSync
import com.example.croft.ui.onboarding.OnboardingEnvironment
import com.example.croft.ui.onboarding.OnboardingTags
import com.example.croft.ui.onboarding.fakeFound
import java.io.File
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * S44 Accounts, S45 Account, S60 About and "Try with sample mail" as a person uses them, through the app's own navigation: from
 * Settings (or from S01) over made-up accounts and a recording [RecordingOps] with a real [SecretStore]. The core's own behaviour is
 * tested in `:mailtests` (real core) and `AccountRemovalTest`.
 */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class AccountsScreensTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val dir = createTempDirectory("croft-accounts-ui").toFile()
    private val themeFile = File(dir, "theme.json")
    private val instant = ThemeSettings.Default.copy(animation = AnimationLevel.OFF)
    private val secrets = SecretStore(File(dir, "secrets"), AesGcmSecretBox(MemoryKeySource()))
    private val prefsStore = AccountPrefsStore.onFile(File(dir, "accounts.json"), scope)
    private val mailStore = MailSettingsStore.onFile(File(dir, "mail.json"), scope)
    private val swipeStore = com.example.croft.ui.swipe.SwipeSettingsStore.onFile(File(dir, "swipe.json"), scope)
    private val accounts = testAccounts()
    private val sessionAndRepo = accountsSession(accounts)
    private val session = sessionAndRepo.first
    private val repo = sessionAndRepo.second
    private val ops = RecordingOps(repo, secrets)
    private val backend = FakeBackend()
    private var initialTheme = instant
    private val themeStore: ThemeStore by lazy { themeFile.writeText(ThemeJson.encode(initialTheme)); ThemeStore.onFile(themeFile, scope) }

    @After
    fun tearDown() { scope.cancel(); dir.deleteRecursively() }

    private fun show(startAt: Route = Route.SETTINGS, session: com.example.croft.mail.MailSession = this.session) {
        val env = OnboardingEnvironment(backend, FakeFirstSync()) { session }
        rule.setContent {
            CroftApp(
                themeStore, RustInfo("9.8.7", 6u), session, startAt = startAt, onboarding = env, accountOps = ops,
                accountPrefsStore = prefsStore, mailStore = mailStore, swipeStore = swipeStore,
            )
        }
    }

    private fun tag(t: String) = rule.onNodeWithTag(t)
    private fun exists(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun waitFor(t: String) = rule.waitUntil(10_000) { exists(t) }
    private fun waitGone(t: String) = rule.waitUntil(10_000) { !exists(t) }

    private fun openAccounts() {
        waitFor(SettingsTags.SCREEN)
        tag(SettingsTags.ACCOUNTS).performClick()
        waitFor(AccountsTags.ACCOUNTS)
        waitFor(AccountsTags.row(1))
    }

    private fun openAbout() {
        waitFor(SettingsTags.SCREEN)
        tag(SettingsTags.ABOUT).performScrollTo().performClick()
        waitFor(AccountsTags.ABOUT)
    }

    private fun back() {
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
    }

    // ---- S44 ----

    @Test
    fun accountsListsEveryAccountWithItsStatusChipAndTheSampleMailIsLabelled() {
        show()
        openAccounts()
        for (id in 1L..3L) tag(AccountsTags.row(id)).assertIsDisplayed()
        // The names, the address of a real account, and the status chips as the core reports them.
        rule.onNodeWithText("Sam").assertIsDisplayed()
        rule.onNodeWithText("sam@example.org").assertIsDisplayed()
        // OK for the first account and for the sample mailbox.
        assertEquals(2, rule.onAllNodes(hasText("OK")).fetchSemanticsNodes().size)
        rule.onNodeWithText("Needs sign-in").assertIsDisplayed()
        // The sample mailbox is called what it is, with its own line, and shows no address.
        rule.onNodeWithText("Sample mail").assertIsDisplayed()
        rule.onNodeWithText("Made-up messages. Nothing is sent or saved to a server.").assertIsDisplayed()
        assertTrue(rule.onAllNodes(hasText("robin@example.org")).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun removeShowsWhatWillBeDeletedThenRemovesTheAccountAndItsSecret() {
        secrets.put(2, SecretKind.PASSWORD, "made-up-password".toByteArray())
        show()
        openAccounts()
        tag(AccountsTags.remove(2)).performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        // The list of what is deleted on this device.
        rule.onNodeWithText("Remove Work?").assertIsDisplayed()
        rule.onNodeWithText("Its messages and folders", substring = true).assertExists()
        rule.onNodeWithText("Changes and messages that are waiting to be sent", substring = true).assertExists()
        rule.onNodeWithText("Its drafts and its outbox", substring = true).assertExists()
        rule.onNodeWithText("Its saved password", substring = true).assertExists()
        assertTrue("nothing is removed before the person confirms", ops.removed.isEmpty() && secrets.has(2, SecretKind.PASSWORD))
        rule.onNodeWithText("Remove").performClick()
        waitGone(AccountsTags.row(2))
        assertEquals(listOf(2L), ops.removed.toList())
        assertFalse("the saved password is deleted with the account", secrets.has(2, SecretKind.PASSWORD))
        tag(AccountsTags.row(1)).assertIsDisplayed()
        tag(AccountsTags.row(3)).assertIsDisplayed()
    }

    @Test
    fun removingAnAccountAlsoDeletesItsPrefsAndItsFolderChoices() {
        runBlocking { prefsStore.set(AccountPrefs(order = listOf(2L, 1L, 3L), names = mapOf(1L to "Keep", 2L to "Work two"), colours = mapOf(2L to AccountColour.AMBER, 3L to AccountColour.TERTIARY))) }
        session.folderMapping.remember(2, FolderRole.ARCHIVE, FakeData.folderId(2, FakeData.RECEIPTS))
        session.folderMapping.decline(2, FolderRole.TRASH)
        session.folderMapping.recordRecent(2, FakeData.folderId(2, FakeData.WORK))
        session.folderMapping.remember(1, FolderRole.ARCHIVE, FakeData.folderId(1, FakeData.RECEIPTS))
        show()
        openAccounts()
        tag(AccountsTags.remove(2)).performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Remove").performClick()
        waitGone(AccountsTags.row(2))
        // The prefs are written and waited for before the removal reports its end: nothing of account 2 is left, the others stay.
        val prefs = runBlocking { prefsStore.prefs.first() }
        assertEquals(listOf(1L, 3L), prefs.order)
        assertEquals(mapOf(1L to "Keep"), prefs.names)
        assertEquals(mapOf(3L to AccountColour.TERTIARY), prefs.colours)
        assertEquals(setOf(1L), session.folderMapping.accounts())
        assertNull(session.folderMapping.mapped(2, FolderRole.ARCHIVE))
    }

    @Test
    fun aNameTypedOnTheAccountScreenIsNotWrittenBackWhenTheAccountIsRemoved() {
        show()
        openAccounts()
        tag(AccountsTags.row(2)).performClick()
        waitFor(AccountsTags.ACCOUNT)
        // Typed and not yet committed (no focus change): leaving the screen would commit it, unless the account is gone.
        tag(AccountsTags.NAME).performTextReplacement("Typed late")
        tag(AccountsTags.REMOVE).performScrollTo().performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Remove").performClick()
        waitFor(AccountsTags.ACCOUNTS)
        waitGone(AccountsTags.row(2))
        rule.waitForIdle()
        assertNull("nothing is written under the removed account's id", runBlocking { prefsStore.prefs.first() }.names[2L])
    }

    @Test
    fun aSavedSignInThatIsATokenIsListedAsOne() {
        secrets.put(2, SecretKind.OAUTH_REFRESH, "made-up-token".toByteArray())
        show()
        openAccounts()
        tag(AccountsTags.remove(2)).performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Its saved sign-in", substring = true).assertExists()
        assertTrue(rule.onAllNodes(hasText("Its saved password", substring = true)).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun cancelKeepsTheAccountAndItsPassword() {
        secrets.put(2, SecretKind.PASSWORD, "made-up-password".toByteArray())
        show()
        openAccounts()
        tag(AccountsTags.remove(2)).performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Cancel").performClick()
        rule.waitForIdle()
        assertTrue(ops.removed.isEmpty())
        assertTrue(secrets.has(2, SecretKind.PASSWORD))
        tag(AccountsTags.row(2)).assertIsDisplayed()
    }

    @Test
    fun theSampleMailboxsRemovalListHasNoPasswordLine() {
        show()
        openAccounts()
        tag(AccountsTags.remove(3)).performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Its drafts and its outbox", substring = true).assertExists()
        assertTrue(rule.onAllNodes(hasText("Its saved password", substring = true)).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun aFailedRemovalSaysSoAndKeepsTheAccount() {
        ops.failRemove = true
        show()
        openAccounts()
        tag(AccountsTags.remove(1)).performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Remove").performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Croft couldn't remove the account. Try again.")).fetchSemanticsNodes().isNotEmpty() }
        tag(AccountsTags.row(1)).assertIsDisplayed()
    }

    @Test
    fun removingTheLastAccountReturnsToTheWelcomeScreen() {
        accounts.retainAll { it.id == 1L }
        show()
        openAccounts()
        tag(AccountsTags.remove(1)).performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Remove").performClick()
        waitFor(OnboardingTags.WELCOME)
        tag(OnboardingTags.WELCOME).assertIsDisplayed()
        assertEquals(listOf(1L), ops.removed.toList())
    }

    @Test
    fun reorderMovesAnAccountAndTheOrderIsKept() {
        show()
        openAccounts()
        tag(AccountsTags.down(1)).performClick()
        rule.waitUntil(10_000) { runBlocking { prefsStore.prefs.first() }.order.isNotEmpty() }
        assertEquals(listOf(2L, 1L, 3L), runBlocking { prefsStore.prefs.first() }.order)
        val top2 = tag(AccountsTags.row(2)).getUnclippedBoundsInRoot().top
        val top1 = tag(AccountsTags.row(1)).getUnclippedBoundsInRoot().top
        assertTrue("account 2 is now above account 1", top2 < top1)
    }

    @Test
    fun addAccountOpensTheSignInFlowAtS02AndBackReturnsToTheList() {
        show()
        openAccounts()
        tag(AccountsTags.ADD).performScrollTo().performClick()
        waitFor(OnboardingTags.ADD_ACCOUNT)
        back()
        waitFor(AccountsTags.ACCOUNTS)
    }

    @Test
    fun signInOnAnAccountThatNeedsItOpensTheSignInForItsAddress() {
        // T21: S45's Sign in (and the banner's) goes to S04 for that account; signing in there is the core's "sign in again".
        backend.onDiscover = { e, _ -> fakeFound(e) }
        show()
        openAccounts()
        tag(AccountsTags.row(2)).performScrollTo().performClick()
        waitFor(AccountsTags.ACCOUNT)
        tag(AccountsTags.SIGN_IN_AGAIN).performScrollTo().performClick()
        waitFor(OnboardingTags.SIGN_IN)
        assertEquals(listOf(testAccounts()[1].email), backend.discovers.toList())
    }

    /** From S44: Add account, S02, the address, Continue, the password, Sign in (the scripted backend answers). */
    private fun addFromAccounts(email: String) {
        backend.onDiscover = { e, _ -> fakeFound(e) }
        openAccounts()
        tag(AccountsTags.ADD).performScrollTo().performClick()
        waitFor(OnboardingTags.ADD_ACCOUNT)
        tag(OnboardingTags.EMAIL).performTextInput(email)
        tag(OnboardingTags.CONTINUE).performScrollTo().performClick()
        waitFor(OnboardingTags.SIGN_IN)
        tag(OnboardingTags.PASSWORD).performScrollTo().performTextInput("made-up-pw-1234")
        tag(OnboardingTags.SIGN_IN_BUTTON).performScrollTo().performClick()
    }

    @Test
    fun addingAnAddressThatIsAlreadyAddedFromAccountsSaysSoAndKeepsTheList() {
        // The core refuses the same address on other servers (T17d): the sign-in screen says so in plain words.
        backend.onAdd = { _, _, _ -> throw uniffi.croft_ffi.CoreException.AccountExists() }
        show()
        addFromAccounts("sam@example.org")
        waitFor(OnboardingTags.SIGN_IN_FAILURE)
        rule.onNodeWithText("This address is already added", substring = true).assertExists()
        assertEquals(1, backend.adds.size)
        back(); back() // S04 to S02, and out of the flow to the list
        waitFor(AccountsTags.ACCOUNTS)
        for (id in 1L..3L) tag(AccountsTags.row(id)).assertIsDisplayed()
    }

    @Test
    fun anAccountAddedFromAccountsReturnsToTheList() {
        // Also what a re-sign-in for the same address and servers ends in: the core updates the account and the flow finishes.
        show()
        addFromAccounts("pat@example.org")
        rule.waitUntil(10_000) { exists(OnboardingTags.FIRST_SYNC) }
        rule.onNodeWithText("Continue in background").performScrollTo().performClick()
        waitFor(AccountsTags.ACCOUNTS)
        tag(AccountsTags.row(1)).assertIsDisplayed()
    }

    // ---- S45 ----

    @Test
    fun accountScreenSavesTheNameAndTheColour() {
        show()
        openAccounts()
        tag(AccountsTags.row(1)).performClick()
        waitFor(AccountsTags.ACCOUNT)
        tag(AccountsTags.NAME).performTextReplacement("Home mail")
        tag(AccountsTags.colour(AccountColour.AMBER)).performScrollTo().performClick()
        rule.onNodeWithContentDescription("Back").performClick()
        waitFor(AccountsTags.ACCOUNTS)
        rule.waitUntil(10_000) { runBlocking { prefsStore.prefs.first() }.names[1L] == "Home mail" }
        val prefs = runBlocking { prefsStore.prefs.first() }
        assertEquals(AccountColour.AMBER, prefs.colours[1L])
        rule.onNodeWithText("Home mail").assertIsDisplayed()
    }

    @Test
    fun folderMappingSavesTheChosenFolder() {
        // Account 1 has no Archive folder on its server: the row asks, and the choice is remembered.
        repo.missingFolders += FakeData.folderId(1, FakeData.ARCHIVE)
        show()
        openAccounts()
        tag(AccountsTags.row(1)).performClick()
        waitFor(AccountsTags.ACCOUNT)
        tag(AccountsTags.folder("archive")).performScrollTo()
        // Only the Archive row asks: the sample-like folder set of account 1 has a Junk folder, so no other row says "Not set".
        rule.onNodeWithText("Not set. Tap to choose a folder.").assertExists()
        tag(AccountsTags.folder("archive")).performClick()
        waitFor(MoveTags.SHEET)
        val receipts = FakeData.folderId(1, FakeData.RECEIPTS)
        // Search for it so that the one entry is at the top of the sheet (a small test window shows only the top of the list).
        tag(MoveTags.SEARCH).performTextInput("Receipts")
        rule.waitUntil(10_000) { exists(MoveTags.folder(receipts)) }
        tag(MoveTags.folder(receipts)).performClick()
        rule.waitUntil(10_000) { session.folderMapping.mapped(1, FolderRole.ARCHIVE) == receipts }
        try {
            rule.waitUntil(10_000) { rule.onAllNodes(hasText("Receipts")).fetchSemanticsNodes().isNotEmpty() }
        } catch (e: androidx.compose.ui.test.ComposeTimeoutException) {
            throw AssertionError("the Archive row does not show the chosen folder:\n" + rule.onRoot(useUnmergedTree = true).printToString(), e)
        }
        // A folder the server itself marks (Trash) is used as it is: shown, nothing to choose.
        rule.onNodeWithText("Trash (from your server)").assertExists()
    }

    @Test
    fun removeFromTheAccountScreenListsWhatGoesAndReturnsToTheList() {
        // The list names what the device really holds for the account (the removal list reads the secret store).
        secrets.put(2, SecretKind.PASSWORD, "made-up-password".toByteArray())
        show()
        openAccounts()
        tag(AccountsTags.row(2)).performClick()
        waitFor(AccountsTags.ACCOUNT)
        tag(AccountsTags.REMOVE).performScrollTo().performClick()
        waitFor(AccountsTags.REMOVE_DIALOG)
        rule.onNodeWithText("Its saved password", substring = true).assertExists()
        rule.onNodeWithText("Remove").performClick()
        waitFor(AccountsTags.ACCOUNTS)
        assertEquals(listOf(2L), ops.removed.toList())
        waitGone(AccountsTags.row(2))
    }

    // ---- S60 ----

    @Test
    fun theAboutScreenShowsTheVersionAndTheNoteAboutGoogleMicrosoftAndApple() {
        show()
        openAbout()
        rule.onNodeWithText("Croft 9.8.7").assertIsDisplayed()
        tag(AccountsTags.NOT_AFFILIATED).performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Croft is not affiliated with Google, Microsoft or Apple.", substring = true).assertIsDisplayed()
    }

    @Test
    fun thePrivacyAndSourceLinksFireAViewIntent() {
        show()
        openAbout()
        tag(AccountsTags.PRIVACY).performScrollTo().performClick()
        val privacy = shadowOf(rule.activity).nextStartedActivity
        assertNotNull(privacy)
        assertEquals(Intent.ACTION_VIEW, privacy.action)
        assertEquals(AboutLinks.PRIVACY_POLICY_URL, privacy.data.toString())
        tag(AccountsTags.SOURCE).performScrollTo().performClick()
        val source = shadowOf(rule.activity).nextStartedActivity
        assertEquals(Intent.ACTION_VIEW, source.action)
        assertEquals(AboutLinks.SOURCE_URL, source.data.toString())
    }

    @Test
    fun theLicencesScreenListsTheDependencies() {
        show()
        openAbout()
        tag(AccountsTags.LICENCES).performScrollTo().performClick()
        waitFor(AccountsTags.LICENCES_SCREEN)
        tag(AccountsTags.LICENCE_LIST).assertIsDisplayed()
        rule.onNodeWithText("Android libraries").assertIsDisplayed()
        assertTrue("at least one dependency with its licence is listed", rule.onAllNodes(hasText("Apache-2.0")).fetchSemanticsNodes().isNotEmpty())
        back()
        waitFor(AccountsTags.ABOUT)
    }

    @Test
    fun copyDiagnosticsPutsVersionsAndStatesOnTheClipboardAndNoAddressOrHost() {
        show()
        openAbout()
        tag(AccountsTags.COPY).performScrollTo().performClick()
        rule.waitUntil(10_000) { exists(AccountsTags.COPY) && rule.onAllNodes(hasText("Diagnostics copied")).fetchSemanticsNodes().isNotEmpty() }
        val clipboard = rule.activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val text = clipboard.primaryClip!!.getItemAt(0).text.toString()
        assertTrue(text.contains("App version: 9.8.7"))
        assertTrue(text.contains("FFI API version: 6"))
        assertTrue(text.contains("Accounts: 3"))
        assertTrue(text.contains("needs sign-in"))
        for (word in listOf("@", "example.org", "acme.test", "example.net", "Sam", "Work", "Personal", "Robin", "Inbox", "Sent")) {
            assertFalse("diagnostics must not contain $word", text.contains(word))
        }
    }

    @Test
    fun resetErasesTheAccountsTheSecretsAndTheSettingsAfterConfirmationAndReturnsToTheWelcomeScreen() {
        // Settings that are not the defaults, an account name, and a saved password.
        initialTheme = instant.copy(mode = ModePreference.DARK)
        runBlocking {
            prefsStore.set(AccountPrefs(names = mapOf(1L to "Home mail"))); mailStore.set(MailSettings(undoSeconds = 10))
            swipeStore.set(com.example.croft.swipe.SwipeSettings.Default.copy(startToEnd = com.example.croft.swipe.SwipeAction.ARCHIVE))
        }
        secrets.put(1, SecretKind.PASSWORD, "made-up-password".toByteArray())
        show()
        openAbout()
        tag(AccountsTags.ERASE_ALL).performScrollTo().performClick()
        waitFor(AccountsTags.ERASE_DIALOG)
        rule.onNodeWithText("Every saved password", substring = true).assertExists()
        assertFalse("nothing is erased before the person confirms", ops.erased)
        rule.onNodeWithText("Erase everything").performClick()
        waitFor(OnboardingTags.WELCOME)
        assertTrue(ops.erased)
        assertFalse(secrets.hasAny())
        assertTrue(accounts.isEmpty())
        assertEquals(ThemeSettings.Default, runBlocking { themeStore.settings.first() })
        assertEquals(AccountPrefs(), runBlocking { prefsStore.prefs.first() })
        assertEquals(MailSettings(), runBlocking { mailStore.settings.first() })
        assertEquals(com.example.croft.swipe.SwipeSettings.Default, runBlocking { swipeStore.settings.first() })
    }

    @Test
    fun resetCanBeCancelled() {
        show()
        openAbout()
        tag(AccountsTags.ERASE_ALL).performScrollTo().performClick()
        waitFor(AccountsTags.ERASE_DIALOG)
        rule.onNodeWithText("Cancel").performClick()
        rule.waitForIdle()
        assertFalse(ops.erased)
        tag(AccountsTags.ABOUT).assertIsDisplayed()
    }

    // ---- Try with sample mail ----

    @Test
    fun trySampleMailWithoutNetworkOpensTheInboxWithSampleRowsThenSettingsAndAccounts() {
        // Nothing but the sample account will exist, and the device is offline (the app's airplane-mode flag).
        accounts.retainAll { it.id == 3L }
        runBlocking { session.repository.setNetworkAvailable(false) }
        ops.samples = 0
        show(startAt = Route.ONBOARDING)
        waitFor(OnboardingTags.WELCOME)
        tag(OnboardingTags.SAMPLE_BUTTON).performClick()
        waitFor(InboxTags.SCREEN)
        assertEquals("the core's own sample account was asked for", 1, ops.samples)
        // Its rows say they are not saved to a server.
        // (The row merges its children for screen readers, so the line is looked up in the unmerged tree; the row's spoken text has it too.)
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(com.example.croft.ui.components.ROW_NOT_SAVED_TAG), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        assertTrue(rule.onAllNodes(androidx.compose.ui.test.hasContentDescription("Not saved to server", substring = true)).fetchSemanticsNodes().isNotEmpty())
        tag(InboxTags.MENU_BUTTON).performClick()
        rule.waitForIdle()
        tag("drawer-settings").performScrollTo().performClick()
        waitFor(SettingsTags.SCREEN)
        tag(SettingsTags.ACCOUNTS).performClick()
        waitFor(AccountsTags.row(3))
        rule.onNodeWithText("Sample mail").assertIsDisplayed()
    }

    @Test
    fun ifTheSampleMailCannotBeOpenedTheWelcomeScreenSaysSo() {
        val failing = object : com.example.croft.mail.accounts.AccountOps by ops {
            override suspend fun addSample(): Long = throw IllegalStateException("made-up failure")
        }
        val env = OnboardingEnvironment(FakeBackend(), FakeFirstSync()) { session }
        rule.setContent { CroftApp(themeStore, RustInfo("9.8.7", 6u), session, startAt = Route.ONBOARDING, onboarding = env, accountOps = failing, accountPrefsStore = prefsStore, mailStore = mailStore) }
        waitFor(OnboardingTags.WELCOME)
        tag(OnboardingTags.SAMPLE_BUTTON).performClick()
        waitFor(OnboardingTags.SAMPLE_PROBLEM)
        tag(OnboardingTags.WELCOME).assertIsDisplayed()
    }
}
