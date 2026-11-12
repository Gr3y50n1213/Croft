// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.search

import java.io.File

/**
 * The recent searches of S27 in one small file in the app's no-backup folder (`croft-search-history.json`): on this device only,
 * never backed up, and deleted with everything else by "Erase all Croft data" (which removes every `croft*` file there). One line per
 * query; a file that cannot be read gives an empty history.
 */
class StoredSearchHistory(private val file: File) : InMemorySearchHistory(read(file)) {
    override fun changed() {
        try {
            val tmp = File(file.parentFile, file.name + ".tmp")
            tmp.writeText(items.joinToString("\n") { it.replace('\n', ' ') })
            if (!tmp.renameTo(file)) {
                file.writeText(tmp.readText())
                tmp.delete()
            }
        } catch (e: Exception) {
            // Recent searches are a convenience: a failed write loses them, nothing else.
        }
    }

    companion object {
        const val FILE_NAME = "croft-search-history.json"

        private fun read(file: File): List<String> = try {
            if (file.isFile) file.readLines().map { it.trim() }.filter { it.isNotEmpty() } else emptyList()
        } catch (e: Exception) {
            emptyList()
        }
    }
}
