// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.onboarding.CoreOAuthBackend
import com.example.croft.mail.onboarding.CoreOnboardingBackend
import com.example.croft.mail.onboarding.OAuthPhase
import com.example.croft.mail.onboarding.OAuthProviderChoice
import com.example.croft.mail.onboarding.OAuthSignIn
import java.io.File
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CredentialProvider
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings
import uniffi.croft_ffi.TestImapServer
import uniffi.croft_ffi.TestTokenEndpoint

/** Keeps each account's sign-in secret, as the app's keystore does; counts every hand-over. Values are made up. */
private class SecretKeeper : CredentialProvider {
    val stored = HashMap<Long, ByteArray>()
    val saves = CopyOnWriteArrayList<Long>()

    @Synchronized override fun credential(accountId: Long): FfiCredential? = stored[accountId]?.let { FfiCredential.oauthStored(it.copyOf()) }

    override fun signInNeeded(accountId: Long) {}

    @Synchronized override fun saveTokens(accountId: Long, secret: ByteArray) {
        stored[accountId] = secret.copyOf()
        secret.fill(0)
        saves += accountId
    }
}

/**
 * T21 over the real core (host library): the app's own sign-in controller and onboarding backend, a simulated browser, the fake
 * token endpoint and the in-process IMAP server. The tokens reach only the credential provider: none is in the database file.
 */
class OAuthFlowTest {
    @Test
    fun theSignInAddsAnAccountThatSyncsAndNoTokenIsInTheDatabase() {
        val server = TestImapServer.start(2u, "access-1") // the access token the endpoint gives first
        val endpoint = TestTokenEndpoint.start()
        val dir: File = Files.createTempDirectory("croft-oauth").toFile()
        val keeper = SecretKeeper()
        val core = Core.open(File(dir, "croft.db").path, null, keeper)
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        try {
            val signedIn = CompletableDeferred<ByteArray>()
            val opened = CopyOnWriteArrayList<String>()
            val oauth = OAuthSignIn(
                CoreOAuthBackend { core }, scope,
                openTab = { url -> opened += url; assertTrue(endpoint.completeSignIn(url, "code-1", "refresh-0")) },
                onSignedIn = { signedIn.complete(it) },
            )
            oauth.start(OAuthProviderChoice.GOOGLE, "1234-made-up.apps.example", null, server.username(), "Done")
            val secret = runBlocking { withTimeout(20_000) { signedIn.await() } }
            assertEquals(OAuthPhase.Idle, oauth.phase.value)
            assertTrue(opened.single().startsWith("https://accounts.google.com/o/oauth2/v2/auth?"))
            assertTrue(opened.single().contains("code_challenge_method=S256"))

            val discovered = FfiDiscovered(
                email = "alice@example.org",
                imap = ServerSettings(server.host(), server.port(), SecurityMode.TLS, server.username()),
                smtp = null, jmap = null, source = DiscoverySource.AUTOCONFIG, provider = null, authHint = AuthKind.O_AUTH2,
            )
            val backend = CoreOnboardingBackend(
                core,
                adder = { _, _, _, _, _ -> error("not a password sign-in") },
                oauthAdder = { d, name, s ->
                    val id = core.addAccount(d, name, server.certificateFingerprint(), null, FfiCredential.oauthStored(s.copyOf())).id
                    keeper.stored[id] = s.copyOf()
                    id
                },
            )
            val pin = runBlocking { backend.verifyOAuth(discovered, secret, server.certificateFingerprint()) }
            val id = runBlocking { backend.addOAuthAccount(discovered, "alice", secret) }
            secret.fill(0)
            assertEquals(server.certificateFingerprint().lowercase(), pin?.lowercase())

            await("the Inbox with 2 messages") {
                val inbox = core.listFolders(id).firstOrNull { it.path == "INBOX" } ?: return@await null
                inbox.takeIf { it.totalCount.toInt() == 2 }
            }
            // The verifier went to the endpoint; no client secret did (a Microsoft-style public client).
            val sent = endpoint.requests().joinToString("\n")
            assertTrue(sent.contains("code_verifier="))
            assertFalse(sent.contains("client_secret"))
            // No token or code in the database (this test database is not encrypted, so it would be readable).
            core.close()
            for (file in dir.listFiles()!!.filter { it.name.startsWith("croft.db") }) {
                val text = String(file.readBytes(), Charsets.ISO_8859_1)
                for (value in listOf("refresh-0", "access-1", "code-1")) assertFalse("$value in ${file.name}", text.contains(value))
            }
        } finally {
            scope.cancel()
            endpoint.stop()
            server.stop()
            dir.deleteRecursively()
        }
    }
}
