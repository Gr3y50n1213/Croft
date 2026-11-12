// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import android.content.Context
import com.example.croft.debug.TestHooks
import com.example.croft.mail.accounts.AccountPrefsStore
import com.example.croft.mail.accounts.AccountSweep
import com.example.croft.mail.accounts.AppFiles
import com.example.croft.mail.compose.AttachmentFolder
import com.example.croft.mail.compose.CoreComposeBackend
import com.example.croft.mail.reading.CoreReadingRepository
import com.example.croft.openCore
import com.example.croft.secrets.AccountSecrets
import com.example.croft.secrets.AppSecrets
import com.example.croft.secrets.SecretStoreCredentials
import com.example.croft.secrets.SignInNeeded
import com.example.croft.secrets.createAppSecrets
import com.example.croft.ui.compose.ComposeFiles
import com.example.croft.ui.reading.SharedAttachments
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.Core

/** What the app holds about the open core, for the screens that add and remove accounts (T17). */
class OpenedCore(
    val core: Core,
    val secrets: AppSecrets,
    val accountSecrets: AccountSecrets,
    /** Accounts the core said need a sign-in again (the credential provider's `sign_in_needed`). */
    val signInNeeded: SignInNeeded,
)

/**
 * Opens the real core once per process, off the main thread, with the database key from the Keystore ([openCore], T11) and the
 * credential provider over the app's secret store, and builds the [MailSession] over it, **always over the core**, with or without
 * accounts: "Try with sample mail" is the core's own sample account (T17c), so no made-up Kotlin data is ever shown. If the secure
 * storage cannot be opened (a Keystore that is not ready, data from a newer Croft) the session is the empty stand-in
 * [MailSession.unavailable] and nothing is changed or deleted; the sign-in screens then say why ("Croft couldn't open its secure storage").
 */
object MailRuntime {
    @Volatile private var opened: OpenedCore? = null
    @Volatile private var current: MailSession? = null

    /**
     * Closes the core and forgets it and the session, so the next call opens a new one (the same database file, a database that was
     * deleted by "Erase all Croft data", or a restart in a test). Nothing may use the old core afterwards.
     */
    fun forgetCore() {
        synchronized(this) {
            try { opened?.core?.close() } catch (e: Exception) { /* already closed */ }
            opened = null
            current = null
        }
    }

    /** The open core, or null before [session] ran, or when it could not be opened. */
    val core: OpenedCore? get() = opened

    /** The database file: `noBackupFilesDir/croft.db` (never part of a backup, see the backup rules). */
    fun databaseFile(context: Context): File = File(context.applicationContext.noBackupFilesDir, "croft.db")

    /** The session of this process: over the core, or the empty stand-in when the core cannot be opened. Call from any thread; it works on the IO dispatcher. */
    suspend fun session(context: Context, settings: MailSettingsStore = MailSettingsStore.shared(context)): MailSession = withContext(Dispatchers.IO) {
        val made = current ?: synchronized(this@MailRuntime) {
            current ?: build(context.applicationContext, settings).also { current = it }
        }
        sweepPrefs(context.applicationContext)
        made
    }

    /** Drops the account order, names and colours of accounts the core does not list (see [sweepOrphans]). Only when the core gave its list. */
    private suspend fun sweepPrefs(context: Context) {
        try {
            val known = opened?.core?.listAccounts()?.map { it.id }?.toSet() ?: return
            AccountPrefsStore.shared(context).update { AccountSweep(known, NoSecrets).prefs(it) }
        } catch (e: Exception) {
            // Tried again at the next start.
        }
    }

    private object NoSecrets : AccountSweep.Secrets {
        override fun storedIds() = emptySet<Long>()
        override fun deleteSecrets(id: Long) = Unit
    }

    /**
     * The open core and secret store, opened on the IO dispatcher if they are not yet (T17: the sign-in screens need the core to look
     * up, check and add an account). Throws what opening throws (secure storage not ready, a database from a newer Croft); the
     * screens turn that into "Croft couldn't open its secure storage".
     */
    suspend fun openedCore(context: Context): OpenedCore = withContext(Dispatchers.IO) { openOnce(context.applicationContext) }

    /**
     * Builds the session again after accounts were added or removed (or everything was erased): a fresh one over the core, with empty
     * list caches, so no row of a removed account can show. The old session is simply dropped.
     */
    suspend fun reloadSession(context: Context, settings: MailSettingsStore = MailSettingsStore.shared(context)): MailSession {
        synchronized(this) { current = null }
        return session(context, settings)
    }

    /**
     * The last step of "Erase all Croft data": the core is forgotten (it was closed and its database deleted), and every file of Croft's in
     * the no-backup folder goes ([AppFiles.eraseMailFiles]: the database files, the secrets folder, the quarantine, the folder choices, the
     * list positions). Returns false when a file could not be deleted.
     */
    fun afterErase(context: Context): Boolean {
        forgetCore()
        try { ComposeFiles.directory(context.applicationContext).deleteRecursively() } catch (e: SecurityException) { /* cleared at the next start */ }
        try { SharedAttachments.directory(context.applicationContext).deleteRecursively() } catch (e: SecurityException) { /* cleared at the next start */ }
        return AppFiles.eraseMailFiles(context.applicationContext.noBackupFilesDir).isEmpty()
    }

    /** How many accounts the core lists (the sample mailbox counts); null when that could not be read (the core is not open, or it failed). */
    suspend fun accountCount(context: Context): Int? = withContext(Dispatchers.IO) {
        try { openOnce(context.applicationContext).core.listAccounts().size } catch (e: Exception) { null } catch (e: UnsatisfiedLinkError) { null }
    }

    private fun build(context: Context, settings: MailSettingsStore): MailSession {
        val open = try {
            openOnce(context)
        } catch (e: Exception) {
            // Secure storage not ready, a database from a newer Croft, ...: no text is kept (it could hold a path).
            return MailSession.unavailable()
        } catch (e: UnsatisfiedLinkError) {
            return MailSession.unavailable()
        }
        val mapping = FileFolderMapping(File(context.noBackupFilesDir, "croft-folders.txt"))
        val repository = CoreMessageRepository(open.core, mapping)
        return MailSession(
            repository = repository,
            anchors = FileAnchorStore(File(context.noBackupFilesDir, "croft-list-anchors-core.json")),
            rowActions = CoreRowActions(repository, mapping),
            folderMapping = mapping,
            compose = CoreComposeBackend(open.core),
            reading = CoreReadingRepository(open.core),
            search = com.example.croft.mail.search.CoreSearchBackend(open.core),
        )
    }

    /**
     * Finishes a removal that a crash cut short ([AccountSweep]): saved secrets and the folder choices of ids the core does not list, and
     * the removals that were begun and not finished. Only when the core really gave its list. Housekeeping: it never stops the app from
     * starting. (The prefs are swept by [session], which can wait for the DataStore.)
     */
    private fun sweepOrphans(context: Context, open: OpenedCore) {
        try {
            val sweep = AccountSweep(open.core.listAccounts().map { it.id }.toSet(), SecretsForSweep(open))
            sweep.secrets()
            sweep.folderMapping(FileFolderMapping(File(context.noBackupFilesDir, "croft-folders.txt")))
            FileAnchorStore(File(context.noBackupFilesDir, "croft-list-anchors-core.json")).also { sweep.anchors(it); it.flush() }
        } catch (e: Exception) {
            // Tried again at the next start.
        }
    }

    /**
     * The folder the picker's copies go to (`cacheDir/compose-attach`, never the database's folder): created if missing, emptied of
     * copies a crash left, and registered with the core, which reads attachments only from a file under it. If that fails, attaching
     * files says "could not be attached"; nothing else is affected.
     */
    private fun registerAttachmentFolder(context: Context, core: Core) {
        try {
            val dir = ComposeFiles.directory(context)
            AttachmentFolder.prepare(dir)
            core.setAttachmentDirectory(dir.path)
        } catch (e: Exception) {
            // No attachments in this run; no path or text is kept.
        }
    }

    /**
     * Empties the folder of opened attachments (`cacheDir/shared-attachments`) at the start of the app: what a crash or an earlier run left
     * is not kept. If that fails nothing else is affected; the folder is made again when an attachment is opened.
     */
    private fun clearSharedAttachments(context: Context) {
        try {
            SharedAttachments.prepare(context)
        } catch (e: Exception) {
            // Cleared at the next start.
        }
    }

    private class SecretsForSweep(private val open: OpenedCore) : AccountSweep.Secrets {
        override fun storedIds() = open.secrets.secrets.accountIds()
        override fun deleteSecrets(id: Long) = open.secrets.secrets.removeAccount(id)
    }

    @Synchronized
    private fun openOnce(context: Context): OpenedCore {
        opened?.let { return it }
        val dbFile = databaseFile(context)
        val secrets = TestHooks.secretsFactory?.invoke(context, dbFile) ?: createAppSecrets(context, dbFile)
        val signIn = SignInNeeded()
        val core = openCore(dbFile.path, secrets, SecretStoreCredentials(secrets.secrets, signIn))
        val accountSecrets = AccountSecrets(secrets.secrets, signIn)
        val open = OpenedCore(core, secrets, accountSecrets, signIn)
        registerAttachmentFolder(context, core)
        clearSharedAttachments(context)
        sweepOrphans(context, open)
        return open.also { opened = it }
    }
}
