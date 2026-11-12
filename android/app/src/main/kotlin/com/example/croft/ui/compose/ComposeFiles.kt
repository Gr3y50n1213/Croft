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
     * [uri] (text) as a file to copy, or null when it is not the `content:` URI of another app's provider. A `file:` URI, or any other
     * scheme, is never read: a link or a share must not be able to make Croft read a file of its own (or of the phone) by path.
     */
    fun resolve(context: Context, uri: String): PickedFile? {
        val parsed = try { Uri.parse(uri) } catch (e: RuntimeException) { return null }
        return resolve(context, parsed)
    }

    /** [uri] as a file to copy, or null unless it is a `content:` URI of another app's provider ([isForeign]). */
    fun resolve(context: Context, uri: Uri): PickedFile? =
        if (isForeign(context, uri)) ContentFile(context.contentResolver, uri) else null

    /**
     * Whether [uri] is a `content:` URI served by **another app**. Shared content is accepted only from other apps' providers: a link or a
     * share that names one of Croft's own providers (the FileProvider the reading screens add, which serves Croft's own files) would
     * otherwise make Croft attach its own data to a message.
     *
     * - The scheme must be `content`; a `userId@` prefix in the authority (`0@media`) is stripped before the lookup.
     * - Refused when the authority belongs to this package: the providers of this package are listed from its own package info (which
     *   needs no package visibility), and `resolveContentProvider` is asked for the rest (refused when it names this package or this uid).
     * - Refused for a MediaStore item that this app owns (API 29 and later: `OWNER_PACKAGE_NAME`). On API 26 to 28 MediaStore has no owner
     *   column; Croft saves nothing to MediaStore, so there is nothing of its own to protect there.
     * - An authority that does not resolve is accepted: no app serves it, so a read just fails, and refusing it would also refuse real
     *   shares from apps that package visibility hides from this one.
     */
    fun isForeign(context: Context, uri: Uri): Boolean {
        try {
            if (!uri.scheme.equals(ContentResolver.SCHEME_CONTENT, ignoreCase = true)) return false
            val authority = uri.authority?.substringAfterLast('@')?.takeIf { it.isNotEmpty() } ?: return false
            val pm = context.packageManager
            val own = context.packageName
            if (authority in ownAuthorities(context)) return false
            val info = pm.resolveContentProvider(authority, 0)
            if (info != null && (info.packageName == own || info.applicationInfo?.uid == android.os.Process.myUid())) return false
            if (authority == MEDIA_AUTHORITY && ownsMediaItem(context, uri)) return false
            return true
        } catch (e: RuntimeException) {
            // Anything unexpected while deciding: refuse.
            return false
        }
    }

    /** The authorities of this package's own providers. */
    private fun ownAuthorities(context: Context): Set<String> = try {
        @Suppress("DEPRECATION")
        val info = context.packageManager.getPackageInfo(context.packageName, android.content.pm.PackageManager.GET_PROVIDERS)
        info.providers.orEmpty().flatMap { it.authority.orEmpty().split(';') }.filter { it.isNotEmpty() }.toSet()
    } catch (e: android.content.pm.PackageManager.NameNotFoundException) {
        emptySet()
    }

    private const val MEDIA_AUTHORITY = "media"

    /** Whether this app put the MediaStore item at [uri] there (it can read its own items without any permission). */
    private fun ownsMediaItem(context: Context, uri: Uri): Boolean {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) return false
        return try {
            context.contentResolver.query(uri, arrayOf(android.provider.MediaStore.MediaColumns.OWNER_PACKAGE_NAME), null, null, null)?.use { c ->
                c.moveToFirst() && c.getString(0) == context.packageName
            } ?: false
        } catch (e: RuntimeException) {
            false
        }
    }
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
