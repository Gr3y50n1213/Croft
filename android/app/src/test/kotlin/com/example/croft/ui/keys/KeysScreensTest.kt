// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.keys

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.assert
import com.example.croft.CroftRobolectricRunner
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.keys.FakeKeysBackend
import com.example.croft.mail.keys.KeyState
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.show
import java.time.ZoneOffset
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** S54 and S55 (T42a, Robolectric, in-memory backend and files). Every key here is a made-up record; nothing is real key material. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class KeysScreensTest {
    @get:Rule
    val rule = createComposeRule()

    /** The save and open dialogs, in memory. */
    private class Files(var toOpen: ByteArray? = null) : KeyFiles {
        val saved = ArrayList<Pair<String, String>>()
        override fun save(name: String, text: String, done: (Boolean) -> Unit) {
            saved += name to text
            done(true)
        }

        override fun open(done: (ByteArray?) -> Unit) = done(toOpen)
    }

    private fun waitFor(tag: String) = rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty() }

    // ---- S54 ----

    private var prefs by mutableStateOf(AccountPrefs())
    private val imports = ArrayList<Long>()
    private val opened = ArrayList<Long>()

    private fun showKeys(backend: FakeKeysBackend) {
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            EncryptionKeysScreen(
                backend = backend, prefs = prefs, onPrefsChange = { prefs = it },
                onOpenKey = { opened += it }, onImport = { imports += it }, onBack = {}, zone = ZoneOffset.UTC,
            )
        }
        waitFor(KeysTags.card(2))
    }

    @Test
    fun eachAccountShowsItsKeyOrGenerateAndImport() {
        val backend = FakeKeysBackend()
        runBlocking { backend.generate(1) }
        showKeys(backend)
        rule.onNodeWithTag(KeysTags.status(1)).assert(hasText("Active"))
        rule.onNodeWithTag(KeysTags.status(2)).assert(hasText("No key"))
        rule.onNodeWithTag(KeysTags.open(1)).performClick()
        assertEquals(listOf(1L), opened)
        rule.onNodeWithTag(KeysTags.import(2)).performScrollTo().performClick()
        assertEquals(listOf(2L), imports)
        // Generate makes the key on the phone; the card then says so.
        rule.onNodeWithTag(KeysTags.generate(2)).performScrollTo().performClick()
        rule.waitUntil(5_000) { backend.keys[2]?.isNotEmpty() == true }
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag(KeysTags.status(2)) and hasText("Active")).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithText("Private keys stay on this phone", substring = true).assertExists()
    }

    @Test
    fun theDefaultsForNewMessagesAreKeptPerAccount() {
        val backend = FakeKeysBackend()
        runBlocking { backend.generate(1); backend.generate(2) }
        showKeys(backend)
        rule.onNodeWithTag(KeysTags.sign(1)).assertIsOff()
        rule.onNodeWithTag(KeysTags.sign(1)).performClick()
        rule.onNodeWithTag(KeysTags.encrypt(2)).performScrollTo().performClick()
        rule.waitForIdle()
        assertEquals(setOf(1L), prefs.signByDefault)
        assertEquals(setOf(2L), prefs.encryptByDefault)
        rule.onNodeWithTag(KeysTags.sign(1)).assertIsOn()
        rule.onNodeWithTag(KeysTags.encrypt(1)).assertIsOff()
    }

    @Test
    fun aFailureIsSaidInPlainWords() {
        val backend = FakeKeysBackend()
        showKeys(backend)
        backend.failWith = com.example.croft.mail.keys.KeysProblem.NO_VAULT
        rule.onNodeWithTag(KeysTags.generate(1)).performClick()
        waitFor(KeysTags.MESSAGE)
        rule.onNodeWithTag(KeysTags.MESSAGE).assert(hasText("secure storage is not ready", substring = true))
        assertNull(backend.keys[1])
    }

    // ---- S55 ----

    private fun showKey(backend: FakeKeysBackend, files: Files, paste: String? = null) {
        ScreenshotKit.configure(360, 2600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            KeyDetailScreen(backend = backend, account = 1, files = files, onBack = {}, zone = ZoneOffset.UTC, paste = { paste })
        }
        waitFor(KeysTags.CHOOSE_FILE)
    }

    @Test
    fun theKeyShowsItsFingerprintAndItsPublicPartIsExported() {
        val backend = FakeKeysBackend()
        val key = runBlocking { backend.generate(1) }
        val files = Files()
        showKey(backend, files)
        rule.onNodeWithTag(KeysTags.FINGERPRINT).assert(hasText(key.groupedFingerprint))
        assertEquals(10, key.groupedFingerprint.split(" ").size)
        rule.onNodeWithText("sam@example.org").assertExists()
        rule.onNodeWithTag(KeysTags.EXPORT).performClick()
        rule.waitUntil(5_000) { files.saved.isNotEmpty() }
        assertEquals("sam@example.org.public.asc", files.saved.single().first)
        assertTrue(files.saved.single().second.startsWith("-----BEGIN PGP PUBLIC KEY BLOCK-----"))
        rule.onNodeWithTag(KeysTags.MESSAGE).assert(hasText("Saved"))
    }

    @Test
    fun aBackupNeedsTheSamePassphraseTwiceAndAtLeastEightCharacters() {
        val backend = FakeKeysBackend()
        runBlocking { backend.generate(1) }
        val files = Files()
        showKey(backend, files)
        rule.onNodeWithTag(KeysTags.BACKUP).performClick()
        rule.onNodeWithTag(KeysTags.BACKUP_ONE).performTextInput("short")
        rule.onNodeWithText("Use at least 8 characters.").assertExists()
        rule.onNodeWithTag(KeysTags.BACKUP_SAVE).assertIsNotEnabled()
        rule.onNodeWithTag(KeysTags.BACKUP_ONE).performTextInput(" made-up words")
        rule.onNodeWithTag(KeysTags.BACKUP_TWO).performTextInput("short made-up word")
        rule.onNodeWithText("The two passphrases are not the same.").assertExists()
        rule.onNodeWithTag(KeysTags.BACKUP_SAVE).assertIsNotEnabled()
        rule.onNodeWithTag(KeysTags.BACKUP_TWO).performTextInput("s")
        rule.onNodeWithTag(KeysTags.BACKUP_SAVE).performClick()
        rule.waitUntil(5_000) { files.saved.isNotEmpty() }
        assertEquals(listOf("short made-up words"), backend.backups)
        assertEquals("sam@example.org.private-backup.asc", files.saved.single().first)
        assertTrue(files.saved.single().second.startsWith("-----BEGIN PGP PRIVATE KEY BLOCK-----"))
        // The fields are gone once it is saved.
        assertTrue(rule.onAllNodes(hasTestTag(KeysTags.BACKUP_ONE)).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun removingAsksFirstAndThenOnlyImportIsLeft() {
        val backend = FakeKeysBackend()
        runBlocking { backend.generate(1) }
        showKey(backend, Files())
        rule.onNodeWithTag(KeysTags.REMOVE).performScrollTo().performClick()
        rule.onNodeWithText("Remove this key?").assertExists()
        rule.onNodeWithText("cannot be read here again", substring = true).assertExists()
        rule.onNodeWithTag(KeysTags.REMOVE_CONFIRM).performClick()
        rule.waitUntil(5_000) { backend.keys[1].isNullOrEmpty() }
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag(KeysTags.FINGERPRINT)).fetchSemanticsNodes().isEmpty() }
        rule.onNodeWithText("Import a key").assertExists()
    }

    @Test
    fun aKeyFileIsImportedAndAProtectedOneAsksForItsPassphrase() {
        val backend = FakeKeysBackend()
        val files = Files("PROTECTED -----BEGIN PGP PRIVATE KEY BLOCK----- made up".toByteArray())
        showKey(backend, files)
        rule.onNodeWithTag(KeysTags.CHOOSE_FILE).performClick()
        waitFor(KeysTags.IMPORT_PASSPHRASE)
        rule.onNodeWithTag(KeysTags.IMPORT_PASSPHRASE).performTextInput("not it")
        rule.onNodeWithTag(KeysTags.IMPORT_GO).performClick()
        waitFor(KeysTags.MESSAGE)
        rule.onNodeWithTag(KeysTags.MESSAGE).assert(hasText("passphrase does not open", substring = true))
        assertNull(backend.keys[1])
        // The file waits for another try; the right passphrase opens it.
        rule.onNodeWithTag(KeysTags.IMPORT_PASSPHRASE).performTextClearance()
        rule.onNodeWithTag(KeysTags.IMPORT_PASSPHRASE).performTextInput(backend.importPassphrase)
        rule.onNodeWithTag(KeysTags.IMPORT_GO).performClick()
        waitFor(KeysTags.FINGERPRINT)
        val key = runBlocking { backend.ownKey(1) }
        assertNotNull(key)
        assertEquals(KeyState.ACTIVE, key!!.state)
        assertTrue(key.imported)
        // The file's bytes were handed over and then wiped.
        assertTrue(files.toOpen!!.all { it == 0.toByte() })
    }

    @Test
    fun aPastedPublicKeyIsSomeoneElsesAndAnEmptyClipboardSaysSo() {
        val backend = FakeKeysBackend()
        showKey(backend, Files(), paste = "-----BEGIN PGP PUBLIC KEY BLOCK----- made up")
        rule.onNodeWithTag(KeysTags.PASTE).performClick()
        waitFor(KeysTags.MESSAGE)
        rule.onNodeWithTag(KeysTags.MESSAGE).assert(hasText("someone else's public key", substring = true))
        assertNull(runBlocking { backend.ownKey(1) })
        assertFalse(backend.imported.isEmpty())
    }

    @Test
    fun aFileThatCannotBeReadSaysSo() {
        val backend = FakeKeysBackend()
        showKey(backend, Files(toOpen = null))
        rule.onNodeWithTag(KeysTags.CHOOSE_FILE).performClick()
        waitFor(KeysTags.MESSAGE)
        rule.onNodeWithTag(KeysTags.MESSAGE).assert(hasText("could not be read", substring = true))
    }

    @Test
    fun aSaveThatFinishedWhileTheScreenWasGoneIsReportedWhenItIsBack() {
        val backend = FakeKeysBackend()
        runBlocking { backend.generate(1) }
        val files = object : KeyFiles by Files() {
            var notice: KeyFileNotice? = KeyFileNotice.NOT_SAVED
            override fun takeNotice(): KeyFileNotice? = notice.also { notice = null }
        }
        ScreenshotKit.configure(360, 2600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            KeyDetailScreen(backend = backend, account = 1, files = files, onBack = {}, zone = ZoneOffset.UTC, paste = { null })
        }
        waitFor(KeysTags.MESSAGE)
        rule.onNodeWithTag(KeysTags.MESSAGE).assert(hasText("could not be read or written", substring = true))
    }

    @Test
    fun readingAFileStopsPastTheLimit() {
        assertEquals(3, readAtMost("abc".byteInputStream(), 3)!!.size)
        assertNull(readAtMost(ByteArray(10_000).inputStream(), 9_999))
        // Larger than one read buffer, and growing more than once: every byte arrives, in order.
        val big = ByteArray(50_000) { (it % 251).toByte() }
        assertTrue(big.contentEquals(readAtMost(big.inputStream(), 60_000)))
    }
}
