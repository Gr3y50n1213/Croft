// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import android.content.Context
import com.example.croft.openCore
import com.example.croft.secrets.AccountSecrets
import com.example.croft.secrets.AppSecrets
import com.example.croft.secrets.SecretStoreCredentials
import com.example.croft.secrets.SignInNeeded
import com.example.croft.secrets.createAppSecrets
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

    /** Tests only: makes the app's secrets in place of the Keystore ones (a simulated device has no usable AndroidKeyStore). Null in the app. */
    @Volatile internal var secretsFactory: ((Context, File) -> AppSecrets)? = null

    /** Tests only: closes the core and forgets everything, so the next test starts a fresh process-like state. */
    internal fun resetForTests() {
        synchronized(this) {
            try { opened?.core?.close() } catch (e: Exception) { /* already closed */ }
            opened = null
            current = null
            secretsFactory = null
        }
    }

    /** The open core, or null before [session] ran, or when it could not be opened. */
    val core: OpenedCore? get() = opened

    /** The database file: `noBackupFilesDir/croft.db` (never part of a backup, see the backup rules). */
    fun databaseFile(context: Context): File = File(context.applicationContext.noBackupFilesDir, "croft.db")

    /** The session of this process: over the core, or the empty stand-in when the core cannot be opened. Call from any thread; it works on the IO dispatcher. */
    suspend fun session(context: Context, settings: MailSettingsStore = MailSettingsStore.shared(context)): MailSession = withContext(Dispatchers.IO) {
        current ?: synchronized(this@MailRuntime) {
            current ?: build(context.applicationContext, settings).also { current = it }
        }
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

    /** Whether the core has any account (the sample mailbox counts). False when the core cannot be opened: the app then starts at S01. */
    suspend fun hasAccounts(context: Context): Boolean = withContext(Dispatchers.IO) {
        try { openOnce(context.applicationContext).core.listAccounts().isNotEmpty() } catch (e: Exception) { false } catch (e: UnsatisfiedLinkError) { false }
    }

    /** Deletes the app's own mail state files (folder choices, list positions). Part of "Erase all Croft data". */
    fun clearLocalFiles(context: Context) {
        val dir = context.applicationContext.noBackupFilesDir
        listOf("croft-folders.txt", "croft-list-anchors-core.json", "croft-list-anchors.json").forEach { File(dir, it).delete() }
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
        )
    }

    @Synchronized
    private fun openOnce(context: Context): OpenedCore {
        opened?.let { return it }
        val dbFile = databaseFile(context)
        val secrets = secretsFactory?.invoke(context, dbFile) ?: createAppSecrets(context, dbFile)
        val signIn = SignInNeeded()
        val core = openCore(dbFile.path, secrets, SecretStoreCredentials(secrets.secrets, signIn))
        return OpenedCore(core, secrets, AccountSecrets(secrets.secrets, signIn), signIn).also { opened = it }
    }
}
