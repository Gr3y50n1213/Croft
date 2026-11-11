// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import android.content.Context
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.datastore.core.DataStore
import androidx.datastore.core.DataStoreFactory
import androidx.datastore.core.Serializer
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeSensitivity
import com.example.croft.swipe.SwipeSettings
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull

/** The swipe settings for every composable below the theme: the right action, the left action and the sensitivity (S47). */
val LocalSwipeSettings = staticCompositionLocalOf { SwipeSettings.Default }

/**
 * The stored shape of the swipe settings (`croft-swipe.json`, next to the theme file): the two actions as stored ids
 * (start to end, end to start, so right-to-left mirrors by itself) and the sensitivity. Reading never fails: a corrupt,
 * empty or newer file, and any value this build does not offer (Snooze and Move to until their sheets exist), fall back to the
 * default for that field, so the app always starts and a row never gets an action it cannot run.
 */
object SwipeSettingsJson {
    /** Written by this build. */
    const val VERSION = 1

    /** The canonical file text for [s]. */
    fun encode(s: SwipeSettings): String =
        Json { prettyPrint = true; prettyPrintIndent = "  " }.encodeToString(
            JsonObject.serializer(),
            JsonObject(
                linkedMapOf(
                    "version" to JsonPrimitive(VERSION),
                    "startToEnd" to JsonPrimitive(s.startToEnd.id),
                    "endToStart" to JsonPrimitive(s.endToStart.id),
                    "sensitivity" to JsonPrimitive(s.sensitivity.id),
                ),
            ),
        ) + "\n"

    /** The settings in [text], or the defaults for whatever cannot be read. */
    fun decodeOrDefault(text: String): SwipeSettings {
        val obj = try { Json.parseToJsonElement(text) as? JsonObject } catch (e: Exception) { null } ?: return SwipeSettings.Default
        val version = (obj["version"] as? JsonPrimitive)?.intOrNull ?: VERSION
        if (version > VERSION) return SwipeSettings.Default
        fun action(key: String, default: SwipeAction): SwipeAction {
            val found = SwipeAction.fromId((obj[key] as? JsonPrimitive)?.contentOrNull)
            return if (found != null && found.selectable) found else default
        }
        val d = SwipeSettings.Default
        return SwipeSettings(
            startToEnd = action("startToEnd", d.startToEnd),
            endToStart = action("endToStart", d.endToStart),
            sensitivity = SwipeSensitivity.fromId((obj["sensitivity"] as? JsonPrimitive)?.contentOrNull) ?: d.sensitivity,
        )
    }
}

/** DataStore's view of the file: JSON text. */
object SwipeSettingsSerializer : Serializer<SwipeSettings> {
    override val defaultValue: SwipeSettings = SwipeSettings.Default

    override suspend fun readFrom(input: InputStream): SwipeSettings = SwipeSettingsJson.decodeOrDefault(input.readBytes().toString(Charsets.UTF_8))

    override suspend fun writeTo(t: SwipeSettings, output: OutputStream) { output.write(SwipeSettingsJson.encode(t).toByteArray(Charsets.UTF_8)) }
}

/** Persists [SwipeSettings] in the app's private files (DataStore, like the theme). The settings hold no secrets. */
class SwipeSettingsStore(private val dataStore: DataStore<SwipeSettings>) {
    /** The current settings; emits again after every change. */
    val settings: Flow<SwipeSettings> get() = dataStore.data

    /** Replace everything. */
    suspend fun set(value: SwipeSettings) { dataStore.updateData { value } }

    companion object {
        @Volatile private var shared: SwipeSettingsStore? = null

        /** The one store of this process (DataStore refuses two instances on one file). */
        fun shared(context: Context): SwipeSettingsStore = shared ?: synchronized(this) {
            shared ?: onFile(File(context.applicationContext.filesDir, "datastore/croft-swipe.json"), CoroutineScope(SupervisorJob() + Dispatchers.IO)).also { shared = it }
        }

        /** A store on any file, for tests. */
        fun onFile(file: File, scope: CoroutineScope): SwipeSettingsStore =
            SwipeSettingsStore(DataStoreFactory.create(serializer = SwipeSettingsSerializer, scope = scope, produceFile = { file }))
    }
}
