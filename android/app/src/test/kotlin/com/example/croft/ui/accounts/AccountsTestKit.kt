// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import com.example.croft.mail.FakeData
import com.example.croft.mail.FakeMessageRepository
import com.example.croft.mail.FolderInfo
import com.example.croft.mail.MailSession
import com.example.croft.mail.MessageRepository
import com.example.croft.mail.accounts.AccountOps
import com.example.croft.secrets.SecretKind
import com.example.croft.secrets.SecretStore
import com.example.croft.ui.mail.NOW
import java.util.concurrent.CopyOnWriteArrayList

/**
 * The accounts as the screens of S44, S45 and S60 see them in a test: account 1 is fine, account 2 needs a sign-in, account 3 is the
 * sample mailbox (its rows say they are not saved to a server). Folders come from [FakeData] with a made-up message count, so the
 * removal list has numbers to show. Account names and addresses are made up.
 */
class AccountsRepo(private val base: FakeMessageRepository, val accounts: MutableList<AccountInfo>) : MessageRepository by base {
    val baseRepository: FakeMessageRepository get() = base

    override suspend fun listAccounts(): List<AccountInfo> = synchronized(this) { accounts.toList() }

    override suspend fun listFolders(account: AccountId): List<FolderInfo> = base.listFolders(account).map { it.copy(totalCount = FOLDER_TOTAL) }

    companion object {
        /** The message count every test folder reports. */
        const val FOLDER_TOTAL = 7
    }
}

/** The three accounts of the tests, in core order. */
fun testAccounts(count: Int = 3): MutableList<AccountInfo> {
    val all = FakeData.accounts(NOW)
    val three = listOf(
        all[0],
        all[1].copy(status = AccountState.NEEDS_SIGN_IN),
        all[2].copy(label = "Sample mail", email = "robin@example.org", isDemo = true),
    )
    return three.take(count).toMutableList()
}

/**
 * A session over [accounts]. Every row is the sample account's (account 3) and carries `localOnly`, as the real repository gives the
 * rows of the sample mailbox, so the unified Inbox shows nothing else.
 */
fun accountsSession(accounts: MutableList<AccountInfo> = testAccounts(), rows: Int = 30): Pair<MailSession, AccountsRepo> {
    val data = FakeData.rows(rows, NOW).map { it.copy(account = 3, folder = FakeData.folderId(3, it.folder % 100), localOnly = true) }
    val base = FakeMessageRepository(data, { NOW }, FakeData.accounts(NOW))
    val repo = AccountsRepo(base, accounts)
    return MailSession(repo, clock = { NOW }, zone = java.time.ZoneId.of("UTC")) to repo
}

/**
 * [AccountOps] that records what the screens asked and does what the real one does to the state a test can see: removing deletes the
 * account's saved password from a real [SecretStore] (when given) and the account from the list the screens read. Nothing here is real
 * mail; the real core is tested in `:mailtests` and in `AccountRemovalTest`.
 */
class RecordingOps(private val repo: AccountsRepo, private val secrets: SecretStore? = null) : AccountOps {
    val removed = CopyOnWriteArrayList<AccountId>()
    @Volatile var samples = 0
    @Volatile var erased = false
    @Volatile var failRemove = false

    override suspend fun addSample(): AccountId {
        samples++
        val sample = synchronized(repo) { repo.accounts.firstOrNull { it.isDemo } }
        return sample?.id ?: 3L
    }

    override suspend fun remove(id: AccountId) {
        if (failRemove) throw IllegalStateException("made-up failure")
        secrets?.removeAccount(id)
        removed += id
        synchronized(repo) { repo.accounts.removeAll { it.id == id } }
    }

    override suspend fun eraseEverything() {
        erased = true
        secrets?.clear()
        synchronized(repo) { repo.accounts.clear() }
    }

    fun putPassword(id: AccountId) = secrets?.put(id, SecretKind.PASSWORD, "made-up-password".toByteArray())
}

/** The fake repository under [AccountsRepo], for a test that changes it (a folder that goes missing). */
fun AccountsRepo.base(): FakeMessageRepository = baseRepository
