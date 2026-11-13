// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import android.content.Context
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.datastore.core.DataStore
import androidx.datastore.core.DataStoreFactory
import androidx.datastore.core.Serializer
import com.example.croft.design.CroftTokens
import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.mail.reading.RemoteImages
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.intOrNull

/**
 * The mail settings that exist so far (S49, "Inbox and categories" and the privacy row of S57; the rest is built later):
 * - [undoSeconds]: how long the Undo button of a move, delete or spam stays, and how long the core waits before it sends the change.
 *   3, 5 or 10 seconds (`tokens.json`, `swipe.undoWindowSeconds`). The app keeps the choice and hands it to the core
 *   ([MessageRepository.setUndoWindowMs]) at every start and at every change; the core keeps it in memory.
 * - [remoteImages]: Ask, Known senders or Never (default Ask, docs/ux/customisation.md). Passed to the core on every body load.
 * - [markRead]: Immediately, After 2 s or Never (default After 2 s, docs/ux/customisation.md). The reading screen waits that long.
 */
data class MailSettings(
    val undoSeconds: Int = CroftTokens.Swipe.UndoWindowSeconds.default,
    val remoteImages: RemoteImages = RemoteImages.ASK,
    val markRead: MarkReadDelay = MarkReadDelay.AFTER_2_SECONDS,
    /** T35, S49: the category chips above the Inbox, on or off. */
    val categoriesOn: Boolean = true,
    /** T35, S49: which category chips show (All always shows). */
    val shownCategories: Set<MailCategory> = MailCategory.DEFAULT_SHOWN,
) {
    /** The undo window in milliseconds. */
    val undoWindowMs: Long get() = undoSeconds * 1000L

    companion object {
        /** The choices of S49. */
        val UndoChoices: List<Int> = CroftTokens.Swipe.UndoWindowSeconds.options
    }
}

/** The settings for composables below the app, so the screens show the time they will really have. */
val LocalMailSettings = staticCompositionLocalOf { MailSettings() }

/**
 * The stored shape (`croft-mail.json`): a version, the undo time in seconds, and the keys of the remote-images and mark-read choices. A
 * damaged file, or a value that S49 does not offer, reads as the default, field by field; a file from before the two later fields has them
 * as defaults (the version stays 1: the fields are optional).
 */
object MailSettingsJson {
    /** Written by this build. */
    const val VERSION = 1

    /** The canonical file text. */
    fun encode(s: MailSettings): String = Json { prettyPrint = true; prettyPrintIndent = "  " }.encodeToString(
        JsonObject.serializer(),
        JsonObject(
            linkedMapOf(
                "version" to JsonPrimitive(VERSION),
                "undoSeconds" to JsonPrimitive(s.undoSeconds),
                "remoteImages" to JsonPrimitive(s.remoteImages.key),
                "markRead" to JsonPrimitive(s.markRead.key),
                "categories" to JsonPrimitive(s.categoriesOn),
                "shownCategories" to JsonArray(MailCategory.entries.filter { it in s.shownCategories }.map { JsonPrimitive(it.key) }),
            ),
        ),
    ) + "\n"

    /** The settings in [text], or the defaults for whatever cannot be read. */
    fun decodeOrDefault(text: String): MailSettings {
        val obj = try { Json.parseToJsonElement(text) as? JsonObject } catch (e: Exception) { null } ?: return MailSettings()
        val version = (obj["version"] as? JsonPrimitive)?.intOrNull ?: VERSION
        if (version > VERSION) return MailSettings()
        val seconds = (obj["undoSeconds"] as? JsonPrimitive)?.intOrNull
        val defaults = MailSettings()
        return MailSettings(
            undoSeconds = if (seconds != null && seconds in MailSettings.UndoChoices) seconds else defaults.undoSeconds,
            remoteImages = RemoteImages.fromKey((obj["remoteImages"] as? JsonPrimitive)?.takeIf { it.isString }?.content),
            markRead = MarkReadDelay.fromKey((obj["markRead"] as? JsonPrimitive)?.takeIf { it.isString }?.content),
            categoriesOn = (obj["categories"] as? JsonPrimitive)?.booleanOrNull ?: defaults.categoriesOn,
            shownCategories = (obj["shownCategories"] as? JsonArray)
                ?.mapNotNull { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.let { p -> MailCategory.fromKey(p.content) } }
                ?.toSet()
                ?: defaults.shownCategories,
        )
    }
}

/** DataStore's view of the file. */
object MailSettingsSerializer : Serializer<MailSettings> {
    override val defaultValue: MailSettings = MailSettings()

    override suspend fun readFrom(input: InputStream): MailSettings = MailSettingsJson.decodeOrDefault(input.readBytes().toString(Charsets.UTF_8))

    override suspend fun writeTo(t: MailSettings, output: OutputStream) { output.write(MailSettingsJson.encode(t).toByteArray(Charsets.UTF_8)) }
}

/** Persists [MailSettings] in the app's private files (DataStore, like the theme and the swipe settings). No secrets. */
class MailSettingsStore(private val dataStore: DataStore<MailSettings>) {
    /** The current settings; emits again after every change. */
    val settings: Flow<MailSettings> get() = dataStore.data

    /** Replace everything. */
    suspend fun set(value: MailSettings) { dataStore.updateData { value } }

    companion object {
        @Volatile private var shared: MailSettingsStore? = null

        /** The one store of this process. */
        fun shared(context: Context): MailSettingsStore = shared ?: synchronized(this) {
            shared ?: onFile(File(context.applicationContext.filesDir, "datastore/croft-mail.json"), CoroutineScope(SupervisorJob() + Dispatchers.IO)).also { shared = it }
        }

        /** A store on any file, for tests. */
        fun onFile(file: File, scope: CoroutineScope): MailSettingsStore =
            MailSettingsStore(DataStoreFactory.create(serializer = MailSettingsSerializer, scope = scope, produceFile = { file }))
    }
}
