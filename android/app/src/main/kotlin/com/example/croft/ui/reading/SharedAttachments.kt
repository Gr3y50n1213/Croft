// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.net.Uri
import android.webkit.MimeTypeMap
import androidx.activity.ComponentActivity
import androidx.core.content.FileProvider
import java.io.File

/**
 * Where an opened attachment lives and how another app is allowed to read it (store rule, docs/tasks/T18): the core saves the file into
 * [directory] (`cacheDir/shared-attachments`, never the compose-attach folder, never the database's folder), and the viewer gets a
 * `content:` URI of Croft's own `FileProvider` with a **temporary read grant** for that one file. Never a `file://` URI.
 *
 * - The provider is declared in the manifest not exported, with `grantUriPermissions`, and its paths file
 *   (`res/xml/shared_attachment_paths.xml`) exposes **only** `shared-attachments/`: any other file of Croft has no URI.
 * - The folder is emptied at the start of the app ([prepare]), when the viewer returns, and when the conversation screen is left.
 */
object SharedAttachments {
    /** The folder under `cacheDir`, and the only path the provider exposes. */
    const val DIRECTORY_NAME = "shared-attachments"

    /** The provider's authority is the package name and this. Compose refuses `content:` URIs of this authority as share input. */
    const val AUTHORITY_SUFFIX = ".attachments"

    /** The provider's authority in [context]'s app. */
    fun authority(context: Context): String = context.packageName + AUTHORITY_SUFFIX

    /** The folder the core saves attachments to for opening. It holds nothing else. */
    fun directory(context: Context): File = File(context.applicationContext.cacheDir, DIRECTORY_NAME)

    /** Creates the folder if it is missing and empties it (what a crash or an earlier run left). False when that failed: opening an attachment then says so. */
    fun prepare(context: Context): Boolean = try {
        val dir = directory(context)
        clearContents(dir)
        dir.isDirectory || dir.mkdirs()
    } catch (e: SecurityException) {
        false
    }

    /** Deletes what is in [dir], keeping the folder. */
    fun clearContents(dir: File) {
        dir.listFiles()?.forEach { it.deleteRecursively() }
    }

    /** The `content:` URI of [file] through Croft's FileProvider, or null when the file is not under [directory] (the provider exposes nothing else). */
    fun uriFor(context: Context, file: File): Uri? = try {
        FileProvider.getUriForFile(context, authority(context), file)
    } catch (e: IllegalArgumentException) {
        null
    }

    /**
     * The view intent for [uri]: `ACTION_VIEW` with the MIME type, a temporary read grant (`FLAG_GRANT_READ_URI_PERMISSION`, and the URI in
     * the clip data so that the grant reaches the app the person picks in the chooser) and nothing else: no write grant, no extras.
     */
    fun viewIntent(uri: Uri, mimeType: String): Intent = Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uri, safeMimeType(mimeType))
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        clipData = ClipData.newRawUri("", uri)
    }

    /** The browser-style chooser around [view] (the person picks the app each time). */
    fun chooser(context: Context, view: Intent, title: String): Intent = Intent.createChooser(view, title).apply {
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        clipData = view.clipData
        // From a non-activity context (a test) the intent needs its own task.
        if (generateSequence(context) { (it as? ContextWrapper)?.baseContext }.none { it is ComponentActivity }) addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    /** A MIME type of the form `type/subtype` in lower case, else `application/octet-stream`: the type in a message is the sender's claim. */
    fun safeMimeType(claimed: String): String {
        val plain = claimed.substringBefore(';').trim().lowercase()
        return if (MIME.matches(plain) && plain != "*/*") plain else FALLBACK_MIME
    }

    /**
     * The type to open a saved file with. The viewer that reads Croft's FileProvider URI sees a type worked out from the file's
     * extension, so the intent must say the same: the type for [fileName]'s extension ([typeForExtension]) when the sender's [claimed]
     * type agrees with it, otherwise `application/octet-stream` (the chooser then offers apps that open any file, and nothing is
     * rendered as a type the sender only claimed).
     */
    fun viewType(fileName: String, claimed: String, typeForExtension: (String) -> String?): String {
        val ext = fileName.substringAfterLast('.', missingDelimiterValue = "").lowercase()
        val byName = ext.takeIf { it.isNotEmpty() }?.let(typeForExtension)?.let(::safeMimeType) ?: return FALLBACK_MIME
        return if (byName == safeMimeType(claimed)) byName else FALLBACK_MIME
    }

    private val MIME = Regex("""[a-z0-9][a-z0-9!#$&^_.+-]*/[a-z0-9][a-z0-9!#$&^_.+-]*""")
    private const val FALLBACK_MIME = "application/octet-stream"
}

/** Opens a saved attachment in another app. The default opens a chooser; tests record the call. */
fun interface AttachmentOpener {
    /** Returns false when no app can open it (the screen then says so). */
    fun open(file: File, mimeType: String): Boolean
}

/** The real opener: Croft's FileProvider URI with a temporary read grant, in a chooser. */
class ChooserAttachmentOpener(private val context: Context, private val chooserTitle: String) : AttachmentOpener {
    override fun open(file: File, mimeType: String): Boolean {
        val uri = SharedAttachments.uriFor(context, file) ?: return false
        val type = SharedAttachments.viewType(file.name, mimeType) { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) }
        val intent = SharedAttachments.chooser(context, SharedAttachments.viewIntent(uri, type), chooserTitle)
        return try {
            context.startActivity(intent)
            true
        } catch (e: ActivityNotFoundException) {
            false
        } catch (e: SecurityException) {
            false
        }
    }
}
