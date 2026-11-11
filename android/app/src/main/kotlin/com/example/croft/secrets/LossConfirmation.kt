// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit

/**
 * Remembers a *suspected* key loss across app starts. The wrapping key counts as lost on weak
 * evidence ([SecretBoxException.weak]) only when the suspicion has been seen at [requiredStarts]
 * app starts (one per process) spread over at least [minSpanMs]; until then the open fails
 * NOT_READY and nothing is touched. A start that opens the key normally clears the record (the
 * suspicion was a false alarm). The record is a small file: the time of the first sighting and the
 * number of starts that saw it. Nothing in it is secret.
 *
 * The count goes up once per process however many times [observe] is called.
 */
internal class LossConfirmation(
    private val file: File,
    private val clock: () -> Long,
    private val requiredStarts: Int = REQUIRED_STARTS,
    private val minSpanMs: Long = TimeUnit.HOURS.toMillis(MIN_SPAN_HOURS),
) {
    private var countedThisProcess = false

    /** Records this start's sighting (once per process); true when the suspicion has persisted long enough. */
    @Synchronized
    fun observe(): Boolean {
        val now = clock()
        val (first, recorded) = read() ?: (now to 0)
        var count = recorded
        if (!countedThisProcess || !file.exists()) {
            count++
            countedThisProcess = true
            try {
                writeAtomically(file, "$first $count".toByteArray())
            } catch (e: IOException) {
                // Without a record nothing can persist: the answer stays "not yet".
                return false
            }
        }
        return count >= requiredStarts && now - first >= minSpanMs
    }

    /** The key opened normally (or a new one was made): forget the suspicion. */
    @Synchronized
    fun clear() {
        countedThisProcess = false
        file.delete()
    }

    /** The sightings recorded so far (0 when none). */
    @Synchronized
    fun count(): Int = read()?.second ?: 0

    private fun read(): Pair<Long, Int>? = try {
        val parts = file.readText().trim().split(' ')
        if (parts.size == 2) parts[0].toLong() to parts[1].toInt() else null
    } catch (e: IOException) {
        null
    } catch (e: NumberFormatException) {
        null
    }

    override fun toString(): String = "[redacted]"

    internal companion object {
        /** App starts that must each have seen the suspicion. */
        const val REQUIRED_STARTS = 3

        /** Hours between the first and the last sighting. */
        const val MIN_SPAN_HOURS = 1L
    }
}
