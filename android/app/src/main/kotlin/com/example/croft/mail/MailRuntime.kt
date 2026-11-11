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
 * credential provider over the app's secret store, and builds the [MailSession] over it. **While no account exists the app keeps
 * showing the sample mailbox** ([MailSession.shared]): onboarding (T17) is what adds the first account, and until it exists there
 * is nothing else to show. If the secure storage cannot be opened (a Keystore that is not ready, data from a newer Croft) the
 * sample mailbox shows too and nothing is changed or deleted; T17 owns the screen that says why and offers "try again".
 */
object MailRuntime {
    @Volatile private var opened: OpenedCore? = null
    @Volatile private var current: MailSession? = null

    /** The open core, or null before [session] ran, or when it could not be opened. */
    val core: OpenedCore? get() = opened

    /** The database file: `noBackupFilesDir/croft.db` (never part of a backup, see the backup rules). */
    fun databaseFile(context: Context): File = File(context.applicationContext.noBackupFilesDir, "croft.db")

    /** The session of this process: over the core when it has an account, else the sample mailbox. Call from any thread; it works on the IO dispatcher. */
    suspend fun session(context: Context, settings: MailSettingsStore = MailSettingsStore.shared(context)): MailSession = withContext(Dispatchers.IO) {
        current ?: synchronized(this@MailRuntime) {
            current ?: build(context.applicationContext, settings).also { current = it }
        }
    }

    private fun build(context: Context, settings: MailSettingsStore): MailSession {
        val demo = MailSession.shared(context)
        val open = try {
            openOnce(context)
        } catch (e: Exception) {
            // Secure storage not ready, a database from a newer Croft, ...: no text is kept (it could hold a path). The sample mailbox shows.
            return demo
        } catch (e: UnsatisfiedLinkError) {
            return demo
        }
        val accounts = try { open.core.listAccounts() } catch (e: Exception) { return demo }
        if (accounts.isEmpty()) return demo
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
        val secrets = createAppSecrets(context, dbFile)
        val signIn = SignInNeeded()
        val core = openCore(dbFile.path, secrets, SecretStoreCredentials(secrets.secrets, signIn))
        return OpenedCore(core, secrets, AccountSecrets(secrets.secrets, signIn), signIn).also { opened = it }
    }
}
