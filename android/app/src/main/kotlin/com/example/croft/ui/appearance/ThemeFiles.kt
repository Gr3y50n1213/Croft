// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import com.example.croft.theme.McuSchemes
import com.example.croft.theme.ThemeJson
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Saves and opens `croft-theme.json` through the system file picker (no storage permission needed). */
interface ThemeFiles {
    /** Ask where to save [text] as croft-theme.json; [onDone] gets true when it was written. */
    fun export(text: String, onDone: (Boolean) -> Unit)

    /** Ask for a file and hand back its text; null when cancelled or unreadable. Reads at most [ThemeJson.MAX_BYTES] + 1 bytes. */
    fun import(onText: (String?) -> Unit)
}

/** The system-picker implementation, or null when there is no activity to launch pickers from (previews, plain tests). */
@Composable
fun rememberThemeFiles(): ThemeFiles? {
    if (LocalActivityResultRegistryOwner.current == null) return null
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var pendingText by remember { mutableStateOf<String?>(null) }
    var exportDone by remember { mutableStateOf<((Boolean) -> Unit)?>(null) }
    var importDone by remember { mutableStateOf<((String?) -> Unit)?>(null) }

    val create = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument(ThemeJson.MIME_TYPE)) { uri: Uri? ->
        val text = pendingText
        val done = exportDone
        pendingText = null
        exportDone = null
        if (uri == null || text == null) {
            done?.invoke(false)
        } else scope.launch {
            val ok = withContext(Dispatchers.IO) {
                runCatching { context.contentResolver.openOutputStream(uri, "wt")?.use { it.write(text.toByteArray(Charsets.UTF_8)) } != null }.getOrDefault(false)
            }
            done?.invoke(ok)
        }
    }
    val open = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        val done = importDone
        importDone = null
        if (uri == null) {
            done?.invoke(null)
        } else scope.launch {
            val text = withContext(Dispatchers.IO) {
                runCatching {
                    context.contentResolver.openInputStream(uri)?.use { it.readNBytes(ThemeJson.MAX_BYTES + 1).toString(Charsets.UTF_8) }
                }.getOrNull()
            }
            done?.invoke(text)
        }
    }
    return remember(create, open) {
        object : ThemeFiles {
            override fun export(text: String, onDone: (Boolean) -> Unit) {
                pendingText = text
                exportDone = onDone
                create.launch(ThemeJson.FILE_NAME)
            }

            override fun import(onText: (String?) -> Unit) {
                importDone = onText
                open.launch(arrayOf(ThemeJson.MIME_TYPE, "text/plain"))
            }
        }
    }
}

private const val PHOTO_EDGE = 128

/** The system photo picker (no permission needed) plus the colour library's palette scoring; null without an activity. */
@Composable
fun rememberPhotoSeedPicker(): PhotoSeedPicker? {
    if (LocalActivityResultRegistryOwner.current == null) return null
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var done by remember { mutableStateOf<((Long?) -> Unit)?>(null) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri: Uri? ->
        val callback = done
        done = null
        if (uri == null) callback?.invoke(null) else scope.launch {
            val seed = withContext(Dispatchers.IO) { runCatching { seedOfImage(context, uri) }.getOrNull() }
            callback?.invoke(seed)
        }
    }
    return remember(launcher) {
        PhotoSeedPicker { onSeed ->
            done = onSeed
            launcher.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
        }
    }
}

private fun seedOfImage(context: android.content.Context, uri: Uri): Long? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) }
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= PHOTO_EDGE && bounds.outHeight / (sample * 2) >= PHOTO_EDGE) sample *= 2
    val bitmap = context.contentResolver.openInputStream(uri)?.use {
        BitmapFactory.decodeStream(it, null, BitmapFactory.Options().apply { inSampleSize = sample })
    } ?: return null
    val small = Bitmap.createScaledBitmap(bitmap, minOf(bitmap.width, PHOTO_EDGE), minOf(bitmap.height, PHOTO_EDGE), true)
    val pixels = IntArray(small.width * small.height)
    small.getPixels(pixels, 0, small.width, 0, 0, small.width, small.height)
    return McuSchemes.seedFromPixels(pixels)
}
