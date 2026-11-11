// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import java.io.IOException
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.TimeUnit

/**
 * Where data goes when the app has decided its key is lost, instead of being deleted: one folder
 * per incident, `<root>/<timestamp in milliseconds>/`, holding `database/`, `secrets/` and `key/`
 * with the files under their own names (the encrypted database and its side files, the wrapped
 * account secrets, the wrapped database key). The data stays encrypted. The point is that a wrong
 * call (a Keystore failure that looked like a loss) can be undone by moving the files back, which
 * is a support step: nothing in the app does it.
 *
 * An incident is deleted for good only when it is at least [retentionMs] old **and** at least
 * [requiredStarts] successful app starts have happened since (see [recordSuccessfulStart]).
 * The folder lives in the app's no-backup folder.
 */
internal class Quarantine(
    private val root: File,
    private val clock: () -> Long,
    private val retentionMs: Long = TimeUnit.DAYS.toMillis(RETENTION_DAYS),
    private val requiredStarts: Int = REQUIRED_STARTS,
) {
    /**
     * Moves every existing file of each group into a new incident folder, in the order given (put
     * the wrapped key last: a crash half way then repeats the decision at the next start), and
     * returns the folder. Throws [IOException] when a move fails; files already moved stay moved.
     */
    fun moveAside(groups: List<Pair<String, List<File>>>): File {
        val incident = newIncidentFolder()
        for ((name, files) in groups) {
            val target = File(incident, name)
            for (file in files) {
                if (!file.exists()) continue
                target.mkdirs()
                move(file, File(target, file.name))
                syncDirectory(file.absoluteFile.parentFile ?: continue)
            }
            if (target.exists()) syncDirectory(target)
        }
        syncDirectory(incident)
        syncDirectory(root)
        return incident
    }

    /** The incident folders now kept, oldest first. */
    fun incidents(): List<File> =
        root.listFiles { f -> f.isDirectory && timestampOf(f) != null }
            ?.sortedBy { timestampOf(it) } ?: emptyList()

    /**
     * Counts one successful start for every incident except [except] (the one made during this
     * very start), then deletes those that are old enough and have seen enough starts. Failures
     * are ignored: this is housekeeping and must never stop the app from starting.
     */
    fun recordSuccessfulStart(except: File?) {
        for (incident in incidents()) {
            if (incident == except) continue
            try {
                val starts = startsOf(incident) + 1
                val born = timestampOf(incident) ?: continue
                if (clock() - born >= retentionMs && starts >= requiredStarts) {
                    incident.deleteRecursively()
                } else {
                    writeAtomically(File(incident, STARTS_FILE), starts.toString().toByteArray())
                }
            } catch (e: IOException) {
                // Try again at the next start.
            }
        }
    }

    /** Successful starts counted for [incident]. */
    fun startsOf(incident: File): Int =
        try {
            File(incident, STARTS_FILE).readText().trim().toInt()
        } catch (e: IOException) {
            0
        } catch (e: NumberFormatException) {
            0
        }

    private fun newIncidentFolder(): File {
        root.mkdirs()
        val stamp = clock()
        var folder = File(root, stamp.toString())
        var n = 1
        while (folder.exists()) folder = File(root, "$stamp-${n++}")
        if (!folder.mkdirs()) throw IOException("cannot create the quarantine folder")
        return folder
    }

    /** The incident's time, from its name (`<ms>` or `<ms>-<n>`); null for any other folder, which is left alone. */
    private fun timestampOf(folder: File): Long? = folder.name.substringBefore('-').toLongOrNull()

    private fun move(from: File, to: File) {
        try {
            Files.move(from.toPath(), to.toPath(), StandardCopyOption.ATOMIC_MOVE)
        } catch (e: AtomicMoveNotSupportedException) {
            Files.move(from.toPath(), to.toPath())
        }
    }

    override fun toString(): String = "[redacted]"

    internal companion object {
        const val RETENTION_DAYS = 7L
        const val REQUIRED_STARTS = 3
        const val FOLDER_NAME = "croft-quarantine"
        private const val STARTS_FILE = "starts"
    }
}
