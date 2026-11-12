// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import com.example.croft.mail.compose.PickedFile
import java.io.File
import java.io.InputStream

/**
 * Where compose meets the system's files. No storage or media permission is ever requested: the system document picker and the photo
 * picker hand over one URI with a temporary read grant, and the content is copied from that URI into [directory]
 * ([com.example.croft.mail.compose.AttachmentIntake]); a share's attached URIs work the same way.
 */
object ComposeFiles {
    /** The folder the picker's copies are written to, registered with `Core.setAttachmentDirectory` right after the core opens. It holds nothing else. */
    fun directory(context: Context): File = File(context.applicationContext.cacheDir, "compose-attach")

    /**
     * [uri] (text) as a file to copy, or null when it is not a `content:` URI. A `file:` URI, or any other scheme, is never read: a
     * link or a share must not be able to make Croft read a file of its own (or of the phone) by path.
     */
    fun resolve(context: Context, uri: String): PickedFile? {
        val parsed = try { Uri.parse(uri) } catch (e: RuntimeException) { return null }
        return resolve(context.contentResolver, parsed)
    }

    /** [uri] as a file to copy, or null unless its scheme is `content`. */
    fun resolve(resolver: ContentResolver, uri: Uri): PickedFile? =
        if (uri.scheme.equals(ContentResolver.SCHEME_CONTENT, ignoreCase = true)) ContentFile(resolver, uri) else null
}

/** A `content:` URI seen as a [PickedFile]. Names and sizes come from the provider's `OpenableColumns` when it has them. */
class ContentFile(private val resolver: ContentResolver, private val uri: Uri) : PickedFile {
    private class Meta(val name: String?, val size: Long?)

    private val meta: Meta by lazy {
        var name: String? = null
        var size: Long? = null
        try {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    val n = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    val s = c.getColumnIndex(OpenableColumns.SIZE)
                    if (n >= 0 && !c.isNull(n)) name = c.getString(n)
                    if (s >= 0 && !c.isNull(s)) size = c.getLong(s).takeIf { it >= 0 }
                }
            }
        } catch (e: RuntimeException) {
            // A provider that refuses to say: the size is unknown and the copy is capped instead.
        }
        Meta(name, size)
    }

    override val displayName: String? get() = meta.name

    override val declaredSize: Long? get() = meta.size

    override val mimeType: String? get() = try { resolver.getType(uri) } catch (e: RuntimeException) { null }

    override fun open(): InputStream? = try { resolver.openInputStream(uri) } catch (e: java.io.IOException) { null } catch (e: RuntimeException) { null }
}
