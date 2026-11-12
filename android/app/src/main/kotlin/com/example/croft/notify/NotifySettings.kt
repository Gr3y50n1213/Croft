// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.core.DataStoreFactory
import androidx.datastore.core.Serializer
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

/** How often the background check runs (S46). 15 minutes is WorkManager's minimum; [MANUAL] schedules nothing. */
enum class CheckFrequency(val key: String, val minutes: Long?) {
    MIN_15("15m", 15),
    MIN_30("30m", 30),
    HOUR_1("1h", 60),
    HOUR_2("2h", 120),
    MANUAL("manual", null),
    ;

    companion object {
        /** The default (docs/tasks/T20): every 30 minutes. */
        val Default = MIN_30
        fun fromKey(key: String?): CheckFrequency = entries.firstOrNull { it.key == key } ?: Default
    }
}

/** A button on a new-mail notification (S32). The person picks up to [NotifySettings.MAX_ACTIONS]. */
enum class NotifyAction(val key: String) {
    ARCHIVE("archive"),
    DELETE("delete"),
    MARK_READ("read"),
    REPLY("reply"),
    ;

    companion object {
        fun fromKey(key: String?): NotifyAction? = entries.firstOrNull { it.key == key }
    }
}

/**
 * The notification settings (S07, S46). Kept in the app's private files, no secrets, nothing personal beyond account row ids.
 * - [enabled]: the master switch. Off, the check still runs (mail stays fresh) but posts nothing.
 * - [mutedAccounts]: accounts whose switch is off.
 * - [frequency], [batteryNotLow]: the periodic work's interval and its optional constraint.
 * - [actions]: the buttons, in this order, at most three (the screens table of S46 says three; recorded in DECISIONS).
 * - [hideOnLockScreen]: the lock screen shows only "N new messages" (default on).
 * - [introDone]: S07 was shown (or skipped) once, so it is not shown again.
 * - [signInNotified]: accounts that already got their one "Needs sign-in" notification; cleared when the account works again.
 */
data class NotifySettings(
    val enabled: Boolean = true,
    val mutedAccounts: Set<Long> = emptySet(),
    val frequency: CheckFrequency = CheckFrequency.Default,
    val batteryNotLow: Boolean = false,
    val actions: List<NotifyAction> = DefaultActions,
    val hideOnLockScreen: Boolean = true,
    val introDone: Boolean = false,
    val signInNotified: Set<Long> = emptySet(),
) {
    /** Whether new mail of [account] gets a notification. */
    fun wants(account: Long): Boolean = enabled && account !in mutedAccounts

    /** The settings with [action] turned on or off; never more than [MAX_ACTIONS], order kept as Archive, Delete, Mark read, Reply. */
    fun withAction(action: NotifyAction, on: Boolean): NotifySettings {
        val next = if (on) (actions + action).distinct() else actions - action
        if (next.size > MAX_ACTIONS) return this
        return copy(actions = NotifyAction.entries.filter { it in next })
    }

    /** The settings without anything about [account] (it was removed). */
    fun forget(account: Long): NotifySettings = copy(mutedAccounts = mutedAccounts - account, signInNotified = signInNotified - account)

    companion object {
        const val MAX_ACTIONS = 3
        val DefaultActions = listOf(NotifyAction.ARCHIVE, NotifyAction.MARK_READ)
    }
}

/** The stored shape (`croft-notify.json`). Whatever cannot be read is the default, field by field. */
object NotifySettingsJson {
    const val VERSION = 1

    fun encode(s: NotifySettings): String = Json { prettyPrint = true; prettyPrintIndent = "  " }.encodeToString(
        JsonObject.serializer(),
        JsonObject(
            linkedMapOf(
                "version" to JsonPrimitive(VERSION),
                "enabled" to JsonPrimitive(s.enabled),
                "mutedAccounts" to JsonArray(s.mutedAccounts.sorted().map { JsonPrimitive(it) }),
                "frequency" to JsonPrimitive(s.frequency.key),
                "batteryNotLow" to JsonPrimitive(s.batteryNotLow),
                "actions" to JsonArray(s.actions.map { JsonPrimitive(it.key) }),
                "hideOnLockScreen" to JsonPrimitive(s.hideOnLockScreen),
                "introDone" to JsonPrimitive(s.introDone),
                "signInNotified" to JsonArray(s.signInNotified.sorted().map { JsonPrimitive(it) }),
            ),
        ),
    ) + "\n"

    fun decodeOrDefault(text: String): NotifySettings {
        val d = NotifySettings()
        val obj = try { Json.parseToJsonElement(text) as? JsonObject } catch (e: Exception) { null } ?: return d
        val version = (obj["version"] as? JsonPrimitive)?.intOrNull ?: VERSION
        if (version > VERSION) return d
        fun bool(name: String, default: Boolean) = (obj[name] as? JsonPrimitive)?.booleanOrNull ?: default
        fun ids(name: String): Set<Long> = (obj[name] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.longOrNull }?.toSet() ?: emptySet()
        val actions = (obj["actions"] as? JsonArray)
            ?.mapNotNull { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content?.let(NotifyAction::fromKey) }
            ?.distinct()
            ?.takeIf { it.size <= NotifySettings.MAX_ACTIONS }
            ?: d.actions
        return NotifySettings(
            enabled = bool("enabled", d.enabled),
            mutedAccounts = ids("mutedAccounts"),
            frequency = CheckFrequency.fromKey((obj["frequency"] as? JsonPrimitive)?.takeIf { it.isString }?.content),
            batteryNotLow = bool("batteryNotLow", d.batteryNotLow),
            actions = NotifyAction.entries.filter { it in actions },
            hideOnLockScreen = bool("hideOnLockScreen", d.hideOnLockScreen),
            introDone = bool("introDone", d.introDone),
            signInNotified = ids("signInNotified"),
        )
    }
}

private object NotifySettingsSerializer : Serializer<NotifySettings> {
    override val defaultValue = NotifySettings()
    override suspend fun readFrom(input: InputStream) = NotifySettingsJson.decodeOrDefault(input.readBytes().toString(Charsets.UTF_8))
    override suspend fun writeTo(t: NotifySettings, output: OutputStream) { output.write(NotifySettingsJson.encode(t).toByteArray(Charsets.UTF_8)) }
}

/** Persists [NotifySettings] (DataStore, like the other settings). Shared by the screens, the worker and the notification buttons. */
class NotifySettingsStore(private val dataStore: DataStore<NotifySettings>) {
    val settings: Flow<NotifySettings> get() = dataStore.data

    suspend fun current(): NotifySettings = dataStore.data.first()

    suspend fun set(value: NotifySettings) { dataStore.updateData { value } }

    suspend fun update(change: (NotifySettings) -> NotifySettings): NotifySettings = dataStore.updateData(change)

    companion object {
        @Volatile private var shared: NotifySettingsStore? = null

        fun shared(context: Context): NotifySettingsStore = shared ?: synchronized(this) {
            shared ?: onFile(File(context.applicationContext.filesDir, "datastore/croft-notify.json"), CoroutineScope(SupervisorJob() + Dispatchers.IO)).also { shared = it }
        }

        fun onFile(file: File, scope: CoroutineScope): NotifySettingsStore =
            NotifySettingsStore(DataStoreFactory.create(serializer = NotifySettingsSerializer, scope = scope, produceFile = { file }))
    }
}
