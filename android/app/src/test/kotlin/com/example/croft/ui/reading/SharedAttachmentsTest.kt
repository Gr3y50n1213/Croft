// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.test.core.app.ApplicationProvider
import com.example.croft.ui.compose.ComposeFiles
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * How an opened attachment reaches another app: through Croft's FileProvider, with a temporary read grant, for files of one folder only
 * (`cacheDir/shared-attachments`); never `file://`. And that compose refuses to take Croft's own provider's URIs as share input.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SharedAttachmentsTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val made = mutableListOf<File>()

    @After
    fun tearDown() = made.forEach { it.deleteRecursively() }

    private fun fileIn(dir: File, name: String = "report.pdf"): File {
        dir.mkdirs()
        return File(dir, name).also { it.writeText("test"); made += it }
    }

    private fun sharedFile(name: String = "report.pdf") = fileIn(SharedAttachments.directory(context), name)

    @Test
    fun theProviderIsDeclaredNotExported_withGrantUriPermissions_andOurAuthority() {
        val provider = context.packageManager.getProviderInfo(
            android.content.ComponentName(context.packageName, "androidx.core.content.FileProvider"), PackageManager.GET_META_DATA,
        )
        assertFalse("not exported", provider.exported)
        assertTrue("temporary grants only", provider.grantUriPermissions)
        assertEquals(SharedAttachments.authority(context), provider.authority)
        assertEquals(context.packageName + ".attachments", provider.authority)
    }

    @Test
    fun theUriIsAContentUriOfTheProvider_neverFile() {
        val uri = SharedAttachments.uriFor(context, sharedFile())
        assertNotNull(uri)
        assertEquals("content", uri!!.scheme)
        assertEquals(SharedAttachments.authority(context), uri.authority)
        assertTrue("only the shared-attachments folder is named in the path", uri.pathSegments.first() == "shared_attachments")
        assertEquals("report.pdf", uri.lastPathSegment)
    }

    @Test
    fun theViewIntentCarriesATemporaryReadGrant_theMimeType_andNoWriteGrant() {
        val uri = SharedAttachments.uriFor(context, sharedFile())!!
        val view = SharedAttachments.viewIntent(uri, "application/pdf")
        assertEquals(Intent.ACTION_VIEW, view.action)
        assertEquals(uri, view.data)
        assertEquals("application/pdf", view.type)
        assertTrue(view.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)
        assertEquals("no write grant", 0, view.flags and Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        assertEquals("no persistable grant", 0, view.flags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        assertEquals("the grant reaches the app picked in the chooser", uri, view.clipData!!.getItemAt(0).uri)
        assertNull("no extras", view.extras)
        val chooser = SharedAttachments.chooser(context, view, "Open with")
        assertEquals(Intent.ACTION_CHOOSER, chooser.action)
        @Suppress("DEPRECATION")
        assertEquals(view.data, chooser.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)!!.data)
        assertTrue(chooser.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)
    }

    @Test
    fun onlyTheSharedAttachmentsFolderHasUris_nothingElseOfCroftsOwnFiles() {
        val cache = context.cacheDir
        assertNotNull(SharedAttachments.uriFor(context, sharedFile()))
        assertNull("the compose-attach folder", SharedAttachments.uriFor(context, fileIn(ComposeFiles.directory(context))))
        assertNull("the rest of the cache", SharedAttachments.uriFor(context, fileIn(File(cache, "other"))))
        assertNull("the files folder", SharedAttachments.uriFor(context, fileIn(context.filesDir)))
        assertNull("the no-backup folder (the database)", SharedAttachments.uriFor(context, fileIn(context.noBackupFilesDir, "croft.db")))
        assertNull("a path that climbs out", SharedAttachments.uriFor(context, File(SharedAttachments.directory(context), "../croft.db")))
    }

    @Test
    fun thePathsFileListsOnlySharedAttachments() {
        val text = File("src/main/res/xml/shared_attachment_paths.xml").readText()
        val body = text.replace(Regex("<!--.*?-->", RegexOption.DOT_MATCHES_ALL), "")
        val entries = Regex("""<([a-z-]+-path)\b([^>]*)/>""").findAll(body).toList()
        assertEquals("exactly one path", 1, entries.size)
        assertEquals("cache-path", entries[0].groupValues[1])
        assertTrue(entries[0].groupValues[2].contains("path=\"shared-attachments/\""))
        for (broader in listOf("root-path", "files-path", "external-path", "external-files-path", "external-cache-path", "external-media-path")) {
            assertFalse(broader, body.contains("<$broader"))
        }
        // The same file as the app sees it at run time.
        val parser = context.resources.getXml(com.example.croft.R.xml.shared_attachment_paths)
        val tags = mutableListOf<String>()
        while (parser.eventType != android.content.res.XmlResourceParser.END_DOCUMENT) {
            if (parser.eventType == android.content.res.XmlResourceParser.START_TAG && parser.name != "paths") tags += parser.name + ":" + parser.getAttributeValue(null, "path")
            parser.next()
        }
        assertEquals(listOf("cache-path:shared-attachments/"), tags)
    }

    @Test
    fun theFolderIsEmptiedAtStart_andMadeAgain() {
        val file = sharedFile("left-by-a-crash.pdf")
        val sub = fileIn(File(SharedAttachments.directory(context), "nested"), "x.txt")
        assertTrue(file.exists() && sub.exists())
        assertTrue(SharedAttachments.prepare(context))
        assertTrue(SharedAttachments.directory(context).isDirectory)
        assertEquals(0, SharedAttachments.directory(context).listFiles()!!.size)
    }

    @Test
    fun aClaimedMimeTypeIsOnlyUsedWhenItLooksLikeOne() {
        assertEquals("application/pdf", SharedAttachments.safeMimeType("Application/PDF; name=x.pdf"))
        assertEquals("application/octet-stream", SharedAttachments.safeMimeType("*/*"))
        assertEquals("application/octet-stream", SharedAttachments.safeMimeType(""))
        assertEquals("application/octet-stream", SharedAttachments.safeMimeType("not a type"))
        assertEquals("application/octet-stream", SharedAttachments.safeMimeType("text/<script>"))
    }

    // ---- Compose refuses Croft's own provider ----

    @Test
    fun composesShareIntakeRefusesAContentUriOfCroftsOwnProvider_butTakesAnotherAppsUri() {
        val own = "content://${SharedAttachments.authority(context)}/shared_attachments/report.pdf"
        assertNull("our own authority is refused", ComposeFiles.resolve(context, own))
        assertNull("with a user id in front", ComposeFiles.resolve(context, "content://0@${SharedAttachments.authority(context)}/shared_attachments/report.pdf"))
        assertNull("a file: URI is never read", ComposeFiles.resolve(context, "file://${SharedAttachments.directory(context)}/report.pdf"))
        assertNotNull("another app's provider is fine", ComposeFiles.resolve(context, "content://com.example.otherapp.files/docs/photo.jpg"))
        // The share intent drops the stream of our own provider when it is given the context (T18d's intake), and without one the copy step refuses it.
        val share = Intent(Intent.ACTION_SEND).setType("application/pdf").putExtra(Intent.EXTRA_STREAM, Uri.parse(own))
        val withContext = com.example.croft.ui.compose.ComposeIntents.parse(share, context) as com.example.croft.mail.compose.ComposeStart.Fill
        assertEquals(emptyList<String>(), withContext.fill.attachments)
        val without = com.example.croft.ui.compose.ComposeIntents.parse(share) as com.example.croft.mail.compose.ComposeStart.Fill
        assertTrue(without.fill.attachments.all { ComposeFiles.resolve(context, it) == null })
    }
}
