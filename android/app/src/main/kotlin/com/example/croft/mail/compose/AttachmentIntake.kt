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
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
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
 *    of the original (`att-` and random hex, created new and never following a link). The copy stops at the limit plus one byte
 *    whatever the provider declared.
 * 3. `add_draft_attachment_file(account, draft, path, displayName, mime)`: the core reads that file (and only a file under the
 *    registered folder), the shown and sent name is the cleaned display name.
 * 4. Delete the copy, whatever happened.
 *
 * **A provider that stops answering** (its size query, its stream or its reads) cannot hold anything up: everything that can block runs on
 * a worker, and when no byte has moved for [stallMs] the copy is given up ([AttachmentError.TIMED_OUT]), the stream is closed and the
 * partial file deleted. The same happens at once when the caller is cancelled (Back, a chip's Cancel).
 *
 * No name, path or type is logged.
 */
class AttachmentIntake(
    private val backend: ComposeBackend,
    private val dir: File,
    private val random: SecureRandom = SecureRandom(),
    /** How long a copy may go without a byte (or the first answer) before it is given up. */
    private val stallMs: Long = 30_000,
    private val clockNanos: () -> Long = System::nanoTime,
) {
    /** The outcome of one file. */
    sealed interface Result {
        data class Added(val attachment: DraftAttachment) : Result {
            // The file name is not printed (CLAUDE.md, security rules).
            override fun toString(): String = "Added(id=${attachment.id})"
        }

        data class Refused(val error: AttachmentError) : Result
    }

    private val workers = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** What the worker is doing, seen from the waiting side. */
    private class Task(val copy: File, startNanos: Long) {
        @Volatile var progressNanos: Long = startNanos
        @Volatile var abandoned = false
        @Volatile var stream: InputStream? = null
        @Volatile var outcome: Outcome? = null
        val done = CompletableDeferred<Unit>()
    }

    private sealed interface Outcome {
        data class Copied(val bytes: Long, val name: String?, val type: String?) : Outcome

        data class Refused(val error: AttachmentError) : Outcome
    }

    /**
     * Attach [file] to the draft. [currentCount] and [currentBytes] are what the draft already holds, for the count and total limits.
     * Never blocks the caller's thread; returns [Result.Refused] with [AttachmentError.TIMED_OUT] for a provider that stalls.
     */
    suspend fun add(account: AccountId, draftId: Long, file: PickedFile, currentCount: Int = 0, currentBytes: Long = 0): Result {
        val limits = backend.limits()
        if (currentCount >= limits.maxAttachments.toInt()) return Result.Refused(AttachmentError.TOO_MANY)
        val max = limits.maxAttachmentBytes.toLong()
        val total = limits.maxAttachmentsTotalBytes.toLong()
        val copy = newCopyFile() ?: return Result.Refused(AttachmentError.FAILED)
        val task = Task(copy, clockNanos())
        workers.launch { work(task, file, max, total, currentBytes) }
        try {
            while (!task.done.isCompleted) {
                // `join` with a short wake-up: the watchdog needs a clock of its own, because the worker may be stuck for good.
                withTimeoutOrNull(WATCH_MS) { task.done.await() }
                if (!task.done.isCompleted && (clockNanos() - task.progressNanos) / 1_000_000 > stallMs) {
                    abandon(task)
                    return Result.Refused(AttachmentError.TIMED_OUT)
                }
            }
        } catch (e: CancellationException) {
            abandon(task)
            throw e
        }
        val copied = when (val o = task.outcome) {
            is Outcome.Refused -> { deleteQuietly(copy); return Result.Refused(o.error) }
            is Outcome.Copied -> o
            null -> { deleteQuietly(copy); return Result.Refused(AttachmentError.FAILED) }
        }
        return try {
            Result.Added(withContext(Dispatchers.IO) { backend.addAttachmentFile(account, draftId, copy.path, cleanName(copied.name), cleanType(copied.type)) })
        } catch (e: ComposeFailure) {
            Result.Refused(
                when (e.problem) {
                    ComposeProblem.ATTACHMENT_TOO_LARGE -> AttachmentError.TOO_LARGE
                    ComposeProblem.TOO_MANY_ATTACHMENTS -> AttachmentError.TOO_MANY
                    ComposeProblem.ATTACHMENT_UNREADABLE -> AttachmentError.UNREADABLE
                    else -> AttachmentError.FAILED
                },
            )
        } finally {
            deleteQuietly(copy)
        }
    }

    /** Gives up on [task]: the worker is told, the stream closed (which usually wakes a blocked read), and the worker deletes the partial copy when it gets out. */
    private fun abandon(task: Task) {
        task.abandoned = true
        try { task.stream?.close() } catch (e: IOException) { /* it was stuck anyway */ } catch (e: RuntimeException) { /* so was it */ }
        // If the worker is already out, nobody else deletes the copy.
        if (task.done.isCompleted) deleteQuietly(task.copy)
    }

    /** Everything that can block: the provider's questions, `open` and the reads. */
    private fun work(task: Task, file: PickedFile, max: Long, total: Long, currentBytes: Long) {
        try {
            val declared = file.declaredSize
            task.progressNanos = clockNanos()
            if (task.abandoned) return
            if (declared != null && declared > max) { task.outcome = Outcome.Refused(AttachmentError.TOO_LARGE); return }
            if (declared != null && currentBytes + declared > total) { task.outcome = Outcome.Refused(AttachmentError.TOTAL_TOO_LARGE); return }
            val name = file.displayName
            val type = file.mimeType
            task.progressNanos = clockNanos()
            val input = file.open()
            if (input == null) { task.outcome = Outcome.Refused(AttachmentError.UNREADABLE); return }
            task.stream = input
            if (task.abandoned) { input.close(); return }
            val copied = try {
                input.use { copyCapped(it, task, max) }
            } catch (e: IOException) {
                if (!task.abandoned) task.outcome = Outcome.Refused(AttachmentError.UNREADABLE)
                return
            }
            task.outcome = when {
                task.abandoned -> null
                copied > max -> Outcome.Refused(AttachmentError.TOO_LARGE)
                currentBytes + copied > total -> Outcome.Refused(AttachmentError.TOTAL_TOO_LARGE)
                else -> Outcome.Copied(copied, name, type)
            }
        } catch (e: SecurityException) {
            task.outcome = Outcome.Refused(AttachmentError.UNREADABLE)
        } catch (e: RuntimeException) {
            task.outcome = Outcome.Refused(AttachmentError.UNREADABLE)
        } finally {
            if (task.abandoned) deleteQuietly(task.copy)
            task.done.complete(Unit)
        }
    }

    private fun deleteQuietly(f: File) {
        try { Files.deleteIfExists(f.toPath()) } catch (e: IOException) { /* cleared at the next start */ }
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

    /** Copies at most [max] + 1 bytes, noting each chunk as progress; returns how many were copied (more than [max]: over the limit). */
    private fun copyCapped(input: InputStream, task: Task, max: Long): Long {
        var total = 0L
        Files.newOutputStream(task.copy.toPath(), StandardOpenOption.WRITE, StandardOpenOption.TRUNCATE_EXISTING).use { out ->
            val buffer = ByteArray(BUFFER)
            while (!task.abandoned) {
                val room = (max + 1 - total).coerceAtMost(buffer.size.toLong()).toInt()
                if (room <= 0) break
                val n = input.read(buffer, 0, room)
                if (n < 0) break
                out.write(buffer, 0, n)
                total += n
                task.progressNanos = clockNanos()
            }
        }
        return total
    }

    companion object {
        private const val BUFFER = 64 * 1024
        private const val MAX_NAME = 120
        private const val WATCH_MS = 200L

        /**
         * A display name that is only a name: no folders, no control or format characters (the direction overrides and zero-width
         * characters of Unicode category Cf, line and paragraph separators: nothing that can make a name read differently from what it
         * is), bounded, never empty.
         */
        fun cleanName(name: String?): String {
            val last = name.orEmpty().substringAfterLast('/').substringAfterLast('\\')
            val kept = StringBuilder()
            last.codePoints().forEach { cp ->
                val type = Character.getType(cp)
                val bad = Character.isISOControl(cp) || type == Character.FORMAT.toInt() || type == Character.LINE_SEPARATOR.toInt() ||
                    type == Character.PARAGRAPH_SEPARATOR.toInt() || type == Character.UNASSIGNED.toInt() || type == Character.PRIVATE_USE.toInt() ||
                    type == Character.SURROGATE.toInt()
                if (!bad) kept.appendCodePoint(cp)
            }
            val cleaned = kept.toString().trim()
            val cut = if (cleaned.codePointCount(0, cleaned.length) > MAX_NAME) cleaned.substring(0, cleaned.offsetByCodePoints(0, MAX_NAME)) else cleaned
            return cut.trim().ifBlank { "attachment" }
        }

        /** The media type, or the generic one. */
        fun cleanType(type: String?): String {
            val t = type.orEmpty().trim()
            return if (t.length in 3..100 && t.contains('/') && t.none { it.isISOControl() || it == ' ' }) t else "application/octet-stream"
        }
    }
}
