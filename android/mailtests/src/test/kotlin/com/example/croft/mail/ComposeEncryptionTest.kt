// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.compose.AttachmentIntake
import com.example.croft.mail.compose.AutosaveTiming
import com.example.croft.mail.compose.ComposeBackend
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.ComposeController
import com.example.croft.mail.compose.ComposeProblem
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.ComposeState
import com.example.croft.mail.compose.EncryptionDefaults
import com.example.croft.mail.compose.FakeComposeBackend
import com.example.croft.mail.compose.RecipientField
import com.example.croft.mail.compose.SendPrompt
import com.example.croft.mail.compose.SimpleAddressTools
import java.io.File
import java.nio.file.Files
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * T42b: the compose lock and S24 over the in-memory backend, which refuses to send a message to be encrypted unencrypted, as the
 * core does. The rule tested throughout: nothing the person asked to encrypt goes out unencrypted without their choice.
 */
class ComposeEncryptionTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val dirs = mutableListOf<File>()

    @After
    fun tearDown() {
        scope.cancel()
        dirs.forEach { it.deleteRecursively() }
    }

    private val accounts: suspend () -> List<AccountInfo> = { listOf(AccountInfo(1, "Sam", "sam@example.org", AccountState.OK)) }

    private fun controller(backend: ComposeBackend, defaults: EncryptionDefaults = EncryptionDefaults()) = ComposeController(
        backend = backend, scope = scope, accounts = accounts,
        intake = AttachmentIntake(backend, Files.createTempDirectory("croft-encryption-test").toFile().also { dirs += it }),
        tools = SimpleAddressTools, timing = AutosaveTiming(localMs = 60_000, serverIdleMs = 60_000), background = scope,
        defaults = { defaults },
    )

    private fun keyedFake() = FakeComposeBackend().apply { keyed = setOf("bob@example.net") }

    private suspend fun ComposeController.write(vararg to: String) {
        open(ComposeStart.New(1))
        to.forEach { commitRecipients(RecipientField.TO, it) }
        onSubject("Locked")
        onBody("Words")
    }

    @Test(timeout = 60_000)
    fun theLockAndSignAreSavedWithTheDraftAndTheMessageGoesEncrypted() = runBlocking {
        val fake = keyedFake()
        val c = controller(fake)
        c.write("bob@example.net")
        assertEquals("a key was found for Bob", setOf("bob@example.net"), c.state.value.keyed)
        assertFalse(c.state.value.encrypt)
        c.toggleEncrypt()
        c.toggleSign()
        assertEquals(true to true, fake.flagsOf(c.state.value.draftId!!))
        c.requestSend()
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        assertEquals(listOf(true to true), fake.sentFlags)
    }

    @Test(timeout = 60_000)
    fun theDefaultsStartTheMessageAndTheLockFollowsTheRecipientsUntilTouched() = runBlocking {
        val fake = keyedFake()
        val c = controller(fake, EncryptionDefaults(sign = true, encryptWhenKeys = true))
        c.open(ComposeStart.New(1))
        assertTrue(c.state.value.sign)
        assertFalse("no recipient yet", c.state.value.encrypt)
        c.commitRecipients(RecipientField.TO, "bob@example.net")
        assertTrue("every recipient has a key", c.state.value.encrypt)
        c.commitRecipients(RecipientField.TO, "carol@example.com")
        assertFalse("Carol has none", c.state.value.encrypt)
        c.removeRecipient(RecipientField.TO, c.state.value.to.first { it.address.address == "carol@example.com" })
        assertTrue(c.state.value.encrypt)
        // Once the person touches the lock, it stays as they set it.
        c.toggleEncrypt()
        c.toggleEncrypt()
        c.commitRecipients(RecipientField.TO, "carol@example.com")
        assertTrue(c.state.value.encrypt)
        // Off by default: nothing changes on its own.
        val plain = controller(keyedFake())
        plain.write("bob@example.net")
        assertFalse(plain.state.value.encrypt)
        assertFalse(plain.state.value.sign)
    }

    @Test(timeout = 60_000)
    fun aRecipientWithoutAKeyIsAskedAboutAndCanBeRemoved() = runBlocking {
        val fake = keyedFake()
        val c = controller(fake)
        c.write("bob@example.net", "carol@example.com")
        c.toggleEncrypt()
        c.requestSend()
        assertEquals(SendPrompt.EncryptionCheck(listOf("carol@example.com")), c.state.value.prompt)
        assertTrue("nothing sent", fake.sentFlags.isEmpty())
        assertNull(c.state.value.closed)
        // Remove: Carol leaves the message, nothing is sent until the person sends again.
        c.removeMissing()
        assertEquals(listOf("bob@example.net"), c.state.value.to.map { it.address.address })
        assertNull(c.state.value.prompt)
        assertTrue(fake.sentFlags.isEmpty())
        c.requestSend()
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        assertEquals(listOf(true to false), fake.sentFlags)
    }

    @Test(timeout = 60_000)
    fun sendWithoutEncryptionIsThePersonsChoiceAndIsSavedAsSuch() = runBlocking {
        val fake = keyedFake()
        val c = controller(fake)
        c.write("carol@example.com")
        c.toggleEncrypt()
        val id = c.state.value.draftId!!
        c.requestSend()
        assertTrue(c.state.value.prompt is SendPrompt.EncryptionCheck)
        // Cancel: nothing, and the lock is still on.
        c.cancelPrompt()
        assertTrue(c.state.value.encrypt)
        assertTrue(fake.sentFlags.isEmpty())
        c.requestSend()
        c.confirmUnencrypted()
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        assertEquals(listOf(false to false), fake.sentFlags)
        assertEquals(false to false, fake.flagsOf(id))
    }

    @Test(timeout = 60_000)
    fun noOwnKeyAndBccAreAskedAboutAndABusyCheckSendsNothing() = runBlocking {
        val fake = keyedFake().apply { ownKey = false }
        val c = controller(fake)
        c.write("bob@example.net")
        c.toggleEncrypt()
        c.requestSend()
        assertEquals(SendPrompt.EncryptionNoOwnKey, c.state.value.prompt)
        c.cancelPrompt()
        fake.ownKey = true
        c.commitRecipients(RecipientField.BCC, "bob@example.net")
        c.requestSend()
        assertEquals(SendPrompt.EncryptionBcc, c.state.value.prompt)
        c.cancelPrompt()
        c.removeRecipient(RecipientField.BCC, c.state.value.bcc.single())
        // The core cannot say now (its key storage is busy): nothing is sent, nothing is asked, the person tries again.
        fake.encryptionCheckFails = true
        c.requestSend()
        assertNull(c.state.value.prompt)
        assertEquals(ComposeProblem.FAILED, c.state.value.sendProblem)
        assertEquals(ComposeState.Phase.EDITING, c.state.value.phase)
        assertTrue(fake.sentFlags.isEmpty())
    }

    @Test(timeout = 60_000)
    fun aRefusalByTheCoreIsNeverASilentPlainSend() = runBlocking {
        // The check says yes, the send says no (a key went away in between): the message stays, nothing goes.
        val fake = keyedFake()
        val lying = object : ComposeBackend by fake {
            override suspend fun encryptionCheck(account: AccountId, draftId: Long) =
                uniffi.croft_ffi.EncryptionCheck(ownKey = true, missing = emptyList(), hasBcc = false)
        }
        val c = controller(lying)
        c.write("carol@example.com")
        c.toggleEncrypt()
        c.requestSend()
        assertNull(c.state.value.closed)
        assertEquals(ComposeState.Phase.EDITING, c.state.value.phase)
        assertTrue(fake.sentFlags.isEmpty())
        assertTrue(c.state.value.encrypt)
    }
}
