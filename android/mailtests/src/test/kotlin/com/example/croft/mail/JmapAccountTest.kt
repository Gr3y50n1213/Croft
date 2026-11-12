// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.onboarding.CoreOnboardingBackend
import java.io.File
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.TestJmapServer

/**
 * T22 over the real core (host library): a server that discovery found offers JMAP only; the app's onboarding backend checks the
 * password and adds the account, and the app's own repository shows its Inbox. The login is the test server's made-up one.
 */
class JmapAccountTest {
    @Test
    fun aJmapOnlyServerIsAddedThroughTheOnboardingBackendAndItsInboxShows() {
        val server = TestJmapServer.start()
        server.injectMessage("Hello-over-JMAP")
        val dir: File = Files.createTempDirectory("croft-jmap").toFile()
        val provider = MemoryCredentials()
        val core = Core.open(File(dir, "croft.db").path, null, provider)
        try {
            val found = FfiDiscovered(
                email = server.username(), imap = null, smtp = null, jmap = server.sessionUrl(),
                source = DiscoverySource.AUTOCONFIG, provider = null, authHint = AuthKind.PASSWORD,
            )
            val backend = CoreOnboardingBackend(core, adder = { d, name, pin, confirmed, password ->
                val id = core.addAccount(d, name, pin, confirmed, FfiCredential.password(password.copyOf())).id
                provider.store(id, String(password))
                id
            })
            val password = server.password().toByteArray()
            // A plain-http test server has no certificate: no pin.
            assertNull(runBlocking { backend.verify(found, password, null) })
            val id = runBlocking { backend.addAccount(found, "Alice", null, null, password) }
            val repo = CoreMessageRepository(core, InMemoryFolderMapping())
            val rows = await("the JMAP Inbox") {
                val inbox = runBlocking { repo.listFolders(id) }.firstOrNull { it.role == FolderRole.INBOX } ?: return@await null
                runBlocking { repo.listMessages(MessageScope.Folder(id, inbox.id), null, 50) }.rows.takeIf { it.isNotEmpty() }
            }
            assertEquals(listOf("Hello-over-JMAP"), rows.map { it.subject })
        } finally {
            core.close()
            dir.deleteRecursively()
        }
    }
}
