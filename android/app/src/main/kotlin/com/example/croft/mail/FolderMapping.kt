// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import java.io.File

/**
 * What the app remembers about an account's folders (S45's folder mapping, which has no screen yet), per account:
 *
 * - **A folder to use as Archive, Trash or Spam** when the server has none with that special use (docs/ux/message-list.md,
 *   "Folder rules"): Croft asks once, and [remember]s the answer here. The core only knows special-use folders, so the mapping
 *   is the app's; it turns "Archive" into a plain Move to the mapped folder.
 * - **Recent destinations** of Move to (S30), newest first.
 *
 * It holds folder ids (local row ids) and role names, no mail text and no addresses.
 */
interface FolderMapping {
    /** The folder remembered for [role] (ARCHIVE, TRASH or SPAM) of [account], or null. */
    fun mapped(account: AccountId, role: FolderRole): FolderId?

    /** Remember [folder] as [account]'s [role] folder. */
    fun remember(account: AccountId, role: FolderRole, folder: FolderId)

    /** Forget the choice (the folder is gone, or the person changed their mind). */
    fun forget(account: AccountId, role: FolderRole)

    /** The person declined to use any folder as [role] for [account]: the action is hidden for that account (S45 can undo this). */
    fun decline(account: AccountId, role: FolderRole)

    /** Whether [decline] was called for [account] and [role], and no folder was chosen since. */
    fun isDeclined(account: AccountId, role: FolderRole): Boolean

    /** Forgets everything remembered about [account] (it was removed): its folder choices, its declined roles and its recent destinations. */
    fun forgetAccount(account: AccountId)

    /** The accounts something is remembered for (the start-up sweep drops those the core no longer lists). */
    fun accounts(): Set<AccountId>

    /** The last folders [account]'s mail was moved to, newest first, at most [MAX_RECENT]. */
    fun recents(account: AccountId): List<FolderId>

    /** [folder] was just chosen as a destination for [account]. */
    fun recordRecent(account: AccountId, folder: FolderId)

    companion object {
        /** Recent destinations kept per account. */
        const val MAX_RECENT = 5
    }
}

/** [FolderMapping] in memory (tests, the sample mailbox). */
open class InMemoryFolderMapping : FolderMapping {
    private val roles = HashMap<Pair<AccountId, FolderRole>, FolderId>()
    private val recent = HashMap<AccountId, List<FolderId>>()
    private val declined = HashSet<Pair<AccountId, FolderRole>>()

    @Synchronized override fun decline(account: AccountId, role: FolderRole) { roles.remove(account to role); declined += account to role; changed() }

    @Synchronized override fun isDeclined(account: AccountId, role: FolderRole): Boolean = (account to role) in declined

    @Synchronized override fun mapped(account: AccountId, role: FolderRole): FolderId? = roles[account to role]

    @Synchronized override fun remember(account: AccountId, role: FolderRole, folder: FolderId) { roles[account to role] = folder; declined.remove(account to role); changed() }

    @Synchronized override fun forget(account: AccountId, role: FolderRole) { roles.remove(account to role); changed() }

    @Synchronized override fun forgetAccount(account: AccountId) {
        roles.keys.removeAll { it.first == account }
        declined.removeAll { it.first == account }
        recent.remove(account)
        changed()
    }

    @Synchronized override fun accounts(): Set<AccountId> =
        roles.keys.map { it.first }.toSet() + declined.map { it.first } + recent.keys

    @Synchronized override fun recents(account: AccountId): List<FolderId> = recent[account].orEmpty()

    @Synchronized override fun recordRecent(account: AccountId, folder: FolderId) {
        recent[account] = (listOf(folder) + recent[account].orEmpty().filter { it != folder }).take(FolderMapping.MAX_RECENT)
        changed()
    }

    /** Called (under the lock) after every change; [FileFolderMapping] writes the file here. */
    protected open fun changed() {}

    /** The contents as lines, for [FileFolderMapping]. */
    @Synchronized protected fun lines(): List<String> =
        roles.entries.sortedBy { it.key.first }.map { (k, v) -> "m ${k.first} ${k.second.name} $v" } +
            declined.sortedBy { it.first }.map { (a, role) -> "d $a ${role.name}" } +
            recent.entries.sortedBy { it.key }.map { (a, fs) -> "r $a ${fs.joinToString(",")}" }

    /** Replaces the contents with [text]'s lines; anything that is not a valid line is ignored. */
    @Synchronized protected fun load(text: String) {
        for (line in text.lineSequence()) {
            val parts = line.trim().split(' ')
            try {
                when {
                    parts.size == 4 && parts[0] == "m" -> roles[parts[1].toLong() to FolderRole.valueOf(parts[2])] = parts[3].toLong()
                    parts.size == 3 && parts[0] == "d" -> declined += parts[1].toLong() to FolderRole.valueOf(parts[2])
                    parts.size == 3 && parts[0] == "r" -> recent[parts[1].toLong()] = parts[2].split(',').map { it.toLong() }.take(FolderMapping.MAX_RECENT)
                }
            } catch (e: IllegalArgumentException) {
                // A damaged line (including an unknown role name) is skipped; the rest still loads.
            }
        }
    }
}

/** [FolderMapping] kept in a small text file (the app uses `noBackupFilesDir/croft-folders.txt`); written atomically. */
class FileFolderMapping(private val file: File) : InMemoryFolderMapping() {
    init {
        try { if (file.exists()) load(file.readText()) } catch (e: java.io.IOException) { /* start empty */ }
    }

    override fun changed() {
        val tmp = File(file.parentFile, file.name + ".tmp")
        try {
            tmp.writeText(lines().joinToString("\n", postfix = "\n"))
            if (!tmp.renameTo(file)) { file.delete(); tmp.renameTo(file) }
        } catch (e: java.io.IOException) {
            // The choice then lasts for this run only; nothing else depends on the file.
        }
    }
}
