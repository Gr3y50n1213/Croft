// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import java.io.File

/**
 * The last step of "Erase all Croft data on this device" for the files Croft keeps in the app's no-backup folder: everything whose name
 * starts with `croft` (the database and its `-wal`, `-shm` and `-journal` files `croft.db*`, `croft-secrets/`, `croft-quarantine/`,
 * the folder choices `croft-folders.txt`, the list positions `croft-list-anchors*.json`). Files of other names are not Croft's and stay.
 *
 * The settings stores (theme, swipe, mail, accounts) are DataStore files in `filesDir/datastore/croft-*.json`: DataStore keeps its file
 * open, so they are reset to their defaults through the stores themselves instead of being deleted (the one thing that survives an
 * erase; a file of defaults, the same as a fresh install writes at its first change).
 */
object AppFiles {
    /** Deletes Croft's files in [noBackupDir]. Returns the names that could not be deleted (none when all went). */
    fun eraseMailFiles(noBackupDir: File): List<String> {
        val failed = ArrayList<String>()
        noBackupDir.listFiles { f -> f.name.startsWith("croft") }?.forEach { f ->
            if (!f.deleteRecursively()) failed += f.name
        }
        return failed
    }
}
