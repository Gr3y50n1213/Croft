// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.AccountId
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.StandardOpenOption
import java.security.SecureRandom
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.DraftAttachment

/** A file the person picked (or that a share carried), seen through whatever gives access to it: the system's `ContentResolver` in the app. */
interface PickedFile {
    /** The name the provider gave, or null. Used only as the display name after cleaning; it is never a path. */
    val displayName: String?

    /** The media type the provider gave, or null. */
    val mimeType: String?

    /** The size the provider declared in bytes, or null when it did not say. */
    val declaredSize: Long?

    /** Opens the content. Null when it cannot be opened. The caller closes it. */
    fun open(): InputStream?
}

/** Where the picked files are copied before the core reads them: only the picker's cache copies are ever in this folder. */
object AttachmentFolder {
    /** Creates [dir] if it is missing and clears every old copy in it (a copy left by a crash). */
    fun prepare(dir: File) {
        dir.mkdirs()
        dir.listFiles()?.forEach { f -> try { f.deleteRecursively() } catch (e: SecurityException) { /* kept for the next start */ } }
    }
}

/**
 * Turns a picked file into a draft attachment (docs: "Attachments flow from the picker to the core"):
 *
 * 1. Check `compose_limits().max_attachment_bytes` against the declared size **before anything is copied**; an over-limit file is
 *    refused with [AttachmentError.TOO_LARGE] and nothing is written.
 * 2. Copy the content into [dir] (the folder registered with `Core.setAttachmentDirectory`) under a generated name that holds nothing
 *    of the original (`att-` and random hex), created new and never following a link. The copy stops at the limit plus one byte
 *    whatever the provider declared.
 * 3. `add_draft_attachment_file(account, draft, path, displayName, mime)`: the core reads that file (and only a file under the
 *    registered folder), the shown and sent name is the cleaned display name.
 * 4. Delete the copy, whatever happened.
 *
 * No name, path or type is logged.
 */
class AttachmentIntake(
    private val backend: ComposeBackend,
    private val dir: File,
    private val random: SecureRandom = SecureRandom(),
) {
    /** The outcome of one file. */
    sealed interface Result {
        data class Added(val attachment: DraftAttachment) : Result

        data class Refused(val error: AttachmentError) : Result
    }

    /**
     * Attach [file] to the draft. [currentCount] and [currentBytes] are what the draft already holds, for the count and total limits.
     * Runs on the IO dispatcher.
     */
    suspend fun add(account: AccountId, draftId: Long, file: PickedFile, currentCount: Int = 0, currentBytes: Long = 0): Result = withContext(Dispatchers.IO) {
        val limits = backend.limits()
        if (currentCount >= limits.maxAttachments.toInt()) return@withContext Result.Refused(AttachmentError.TOO_MANY)
        val max = limits.maxAttachmentBytes.toLong()
        val declared = file.declaredSize
        if (declared != null && declared > max) return@withContext Result.Refused(AttachmentError.TOO_LARGE)
        if (declared != null && currentBytes + declared > limits.maxAttachmentsTotalBytes.toLong()) return@withContext Result.Refused(AttachmentError.TOTAL_TOO_LARGE)
        val copy = newCopyFile() ?: return@withContext Result.Refused(AttachmentError.FAILED)
        try {
            val copied = try {
                val input = file.open() ?: return@withContext Result.Refused(AttachmentError.UNREADABLE)
                input.use { copyCapped(it, copy, max) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: IOException) {
                return@withContext Result.Refused(AttachmentError.UNREADABLE)
            } catch (e: SecurityException) {
                return@withContext Result.Refused(AttachmentError.UNREADABLE)
            }
            if (copied > max) return@withContext Result.Refused(AttachmentError.TOO_LARGE)
            if (currentBytes + copied > limits.maxAttachmentsTotalBytes.toLong()) return@withContext Result.Refused(AttachmentError.TOTAL_TOO_LARGE)
            try {
                Result.Added(backend.addAttachmentFile(account, draftId, copy.path, cleanName(file.displayName), cleanType(file.mimeType)))
            } catch (e: ComposeFailure) {
                Result.Refused(
                    when (e.problem) {
                        ComposeProblem.ATTACHMENT_TOO_LARGE -> AttachmentError.TOO_LARGE
                        ComposeProblem.TOO_MANY_ATTACHMENTS -> AttachmentError.TOO_MANY
                        ComposeProblem.ATTACHMENT_UNREADABLE -> AttachmentError.UNREADABLE
                        else -> AttachmentError.FAILED
                    },
                )
            }
        } finally {
            try { Files.deleteIfExists(copy.toPath()) } catch (e: IOException) { /* cleared at the next start */ }
        }
    }

    private fun newCopyFile(): File? {
        dir.mkdirs()
        repeat(3) {
            val bytes = ByteArray(16).also { random.nextBytes(it) }
            val file = File(dir, "att-" + bytes.joinToString("") { "%02x".format(it) })
            try {
                // CREATE_NEW: never opens an existing file or follows a link planted at that name.
                Files.newOutputStream(file.toPath(), StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).close()
                return file
            } catch (e: IOException) {
                // Try another name.
            }
        }
        return null
    }

    /** Copies at most [max] + 1 bytes; returns how many were copied (more than [max] means the content is over the limit). */
    private fun copyCapped(input: InputStream, to: File, max: Long): Long {
        var total = 0L
        Files.newOutputStream(to.toPath(), StandardOpenOption.WRITE, StandardOpenOption.TRUNCATE_EXISTING).use { out ->
            val buffer = ByteArray(BUFFER)
            while (true) {
                val room = (max + 1 - total).coerceAtMost(buffer.size.toLong()).toInt()
                if (room <= 0) break
                val n = input.read(buffer, 0, room)
                if (n < 0) break
                out.write(buffer, 0, n)
                total += n
            }
        }
        return total
    }

    companion object {
        private const val BUFFER = 64 * 1024
        private const val MAX_NAME = 120

        /** A display name that is only a name: no folders, no control characters, bounded, never empty. */
        fun cleanName(name: String?): String {
            val last = name.orEmpty().substringAfterLast('/').substringAfterLast('\\')
            val cleaned = last.filter { !it.isISOControl() && it != '‮' && it != '‭' }.trim().take(MAX_NAME)
            return cleaned.ifBlank { "attachment" }
        }

        /** The media type, or the generic one. */
        fun cleanType(type: String?): String {
            val t = type.orEmpty().trim()
            return if (t.length in 3..100 && t.contains('/') && t.none { it.isISOControl() || it == ' ' }) t else "application/octet-stream"
        }
    }
}
