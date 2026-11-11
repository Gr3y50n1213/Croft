// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import java.io.File
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.fail
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CredentialProvider
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings
import uniffi.croft_ffi.TestImapServer

/** The made-up password of the test server. Not a real secret. */
const val TEST_PASSWORD = "made-up-test-password-7731"

/** The app's credential store, in memory: answers only for ids the test stored, and remembers who was told to sign in. */
class MemoryCredentials : CredentialProvider {
    private val stored = HashMap<Long, String>()
    val signInCalls = CopyOnWriteArrayList<Long>()

    @Synchronized fun store(account: Long, password: String) { stored[account] = password }

    @Synchronized override fun credential(accountId: Long): FfiCredential? = stored[accountId]?.let { FfiCredential.password(it.toByteArray()) }

    override fun signInNeeded(accountId: Long) { signInCalls += accountId }
}

/** Polls [check] every 50 ms until it returns non-null (or true), failing after [timeoutMs]. */
fun <T : Any> await(what: String, timeoutMs: Long = 25_000, check: () -> T?): T {
    val end = System.currentTimeMillis() + timeoutMs
    while (System.currentTimeMillis() < end) {
        check()?.let { return it }
        Thread.sleep(50)
    }
    fail("timed out waiting for: $what")
    error("unreachable")
}

/** Whether a command the server logged changes anything on the server (everything else is reading). */
fun isChangingCommand(command: String): Boolean = listOf("MOVE", "COPY", "STORE", "EXPUNGE", "APPEND").any { command.contains(it) }

/**
 * A real core (the host library of the Rust core, on a temporary database) with one account on a real in-process IMAP server
 * (`TestImapServer`, TLS on 127.0.0.1 with the certificate trusted through its fingerprint, as the app does). The repository
 * under test is the app's own [CoreMessageRepository]. [changes] is every change notice since the start.
 */
class Rig(
    messages: Int = 3,
    undoWindowMs: Long = 1_000,
    password: String = TEST_PASSWORD,
    signInPassword: String = password,
) : AutoCloseable {
    val server: TestImapServer = TestImapServer.start(messages.toUInt(), password)
    val dir: File = Files.createTempDirectory("croft-mailtest").toFile()
    val provider = MemoryCredentials()
    val mapping = InMemoryFolderMapping()
    val changes = CopyOnWriteArrayList<ChangeBatch>()

    lateinit var core: Core
        private set
    lateinit var repo: CoreMessageRepository
        private set
    var account: Long = 0
        private set

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var watcher: Job? = null

    init {
        openCore()
        account = runBlocking {
            core.addAccount(discovered(), "Alice", server.certificateFingerprint(), null, FfiCredential.password(signInPassword.toByteArray())).id
        }
        // What the app does once `addAccount` has returned: store the credential under the new id.
        provider.store(account, signInPassword)
        runBlocking { repo.setUndoWindowMs(undoWindowMs) }
    }

    private fun discovered() = FfiDiscovered(
        email = "alice@example.org",
        imap = ServerSettings(server.host(), server.port(), SecurityMode.TLS, server.username()),
        smtp = null,
        jmap = null,
        source = DiscoverySource.AUTOCONFIG,
        provider = null,
        authHint = AuthKind.PASSWORD,
    )

    private fun openCore() {
        core = Core.open(File(dir, "croft.db").path, null, provider)
        repo = CoreMessageRepository(core, mapping)
        val mine = repo
        // The screen's loop: wait for a change, note it, wait again. It must be cancelled before the core is closed (an
        // unfinished call keeps the core alive), exactly as the screen's coroutine is when the process goes.
        watcher = scope.launch {
            var after = 0L
            while (isActive) {
                val batch = mine.nextChange(after)
                after = batch.seq
                changes += batch
            }
        }
    }

    private fun stopWatching() { runBlocking { watcher?.cancelAndJoin() } }

    /** Process death: the core is closed with whatever is queued, and opened again on the same database (the provider has the credential). */
    fun killAndReopen() {
        stopWatching()
        core.close()
        openCore()
    }

    fun <T> run(block: suspend () -> T): T = runBlocking { block() }

    fun folder(path: String): FolderInfo = await("folder $path") { run { repo.listFolders(account) }.firstOrNull { it.path == path } }

    fun scope(path: String): MessageScope = MessageScope.Folder(account, folder(path).id)

    fun rows(path: String): List<MessageRowData> = run { repo.listMessages(scope(path), null, 500).rows }

    fun inbox(): List<MessageRowData> = rows("INBOX")

    fun awaitInbox(count: Int): List<MessageRowData> = await("$count rows in the Inbox") { inbox().takeIf { it.size == count } }

    /** Lets the first sync finish its last commands, then forgets the command log, so that a test sees only its own. */
    fun settle() {
        Thread.sleep(500)
        server.clearCommands()
    }

    fun failures(): List<OpFailure> = changes.flatMap { it.failures }

    fun status(): AccountState = run { repo.listAccounts() }.first { it.id == account }.status

    fun changingCommands(): List<String> = server.commands().filter(::isChangingCommand)

    override fun close() {
        stopWatching()
        scope.cancel()
        try { core.close() } catch (e: Exception) { /* closing a closed core */ }
        server.stop()
        dir.deleteRecursively()
    }
}
