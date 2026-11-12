// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import java.io.File
import java.util.concurrent.Executors
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.floatOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

/**
 * Where a list was left: the first message on screen and how far below (positive) or above (negative) the top of the list its
 * top edge is, in dp. Never a raw pixel value or a raw item index: pixels change with the display size (rotation, a fold) and
 * indexes change when mail arrives, a message id and a dp offset survive both. [dateMs] lets a restore find the place again
 * when that message has gone (it falls back to the nearest older one) and tells how far down to page.
 */
data class ListAnchor(val messageId: MessageId, val dateMs: Long, val offsetDp: Float)

/** The persisted anchor of each folder (and of the unified Inbox), keyed by [MessageScope.key]. */
interface AnchorStore {
    /** The anchor saved for [scopeKey], or null. */
    fun load(scopeKey: String): ListAnchor?

    /** Remember [anchor] for [scopeKey]; null forgets it (the list was left at the top). */
    fun save(scopeKey: String, anchor: ListAnchor?)

    /** Forgets the anchors of [account]'s folders (the account was removed). The unified Inbox's anchor is a message id and date: it stays. */
    fun forgetAccount(account: AccountId)

    /** The accounts some folder anchor is kept for. */
    fun accounts(): Set<AccountId>
}

/** The account of a folder anchor key (`folder:<account>:<folder>`), or null for another key (the unified Inbox). */
internal fun accountOfAnchorKey(key: String): AccountId? =
    key.split(':').takeIf { it.size == 3 && it[0] == "folder" }?.get(1)?.toLongOrNull()

/** An anchor store that lives as long as the process. */
class InMemoryAnchorStore : AnchorStore {
    private val map = HashMap<String, ListAnchor>()

    @Synchronized override fun load(scopeKey: String): ListAnchor? = map[scopeKey]

    @Synchronized override fun save(scopeKey: String, anchor: ListAnchor?) {
        if (anchor == null) map.remove(scopeKey) else map[scopeKey] = anchor
    }

    @Synchronized override fun forgetAccount(account: AccountId) { map.keys.removeAll { accountOfAnchorKey(it) == account } }

    @Synchronized override fun accounts(): Set<AccountId> = map.keys.mapNotNull { accountOfAnchorKey(it) }.toSet()
}

/**
 * Anchors in a small JSON file (message ids, dates and offsets: no addresses and no text), so a cold start returns to the same
 * place. Reads happen once; writes are queued on a background thread, in order. A file that cannot be read is treated as empty.
 */
class FileAnchorStore(private val file: File) : AnchorStore {
    private val map = HashMap<String, ListAnchor>()
    private var loaded = false
    private val io = Executors.newSingleThreadExecutor { r -> Thread(r, "croft-anchors").apply { isDaemon = true } }

    @Synchronized private fun ensureLoaded() {
        if (loaded) return
        loaded = true
        runCatching {
            if (!file.exists()) return
            for ((key, v) in Json.parseToJsonElement(file.readText()).jsonObject) {
                val o = v.jsonObject
                val id = o["id"]?.jsonPrimitive?.longOrNull ?: continue
                val date = o["date"]?.jsonPrimitive?.longOrNull ?: continue
                val offset = o["offset"]?.jsonPrimitive?.floatOrNull ?: continue
                map[key] = ListAnchor(id, date, offset)
            }
        }
    }

    @Synchronized override fun load(scopeKey: String): ListAnchor? { ensureLoaded(); return map[scopeKey] }

    @Synchronized override fun forgetAccount(account: AccountId) {
        ensureLoaded()
        if (map.keys.removeAll { accountOfAnchorKey(it) == account }) write()
    }

    @Synchronized override fun accounts(): Set<AccountId> { ensureLoaded(); return map.keys.mapNotNull { accountOfAnchorKey(it) }.toSet() }

    @Synchronized override fun save(scopeKey: String, anchor: ListAnchor?) {
        ensureLoaded()
        if (anchor == null) map.remove(scopeKey) else map[scopeKey] = anchor
        write()
    }

    /** Queues the current map to be written. */
    private fun write() {
        val json = JsonObject(map.mapValues { (_, a) ->
            JsonObject(mapOf("id" to JsonPrimitive(a.messageId), "date" to JsonPrimitive(a.dateMs), "offset" to JsonPrimitive(a.offsetDp)))
        }).toString()
        io.execute {
            runCatching {
                file.parentFile?.mkdirs()
                val tmp = File(file.path + ".tmp")
                tmp.writeText(json)
                if (!tmp.renameTo(file)) { file.delete(); tmp.renameTo(file) }
            }
        }
    }

    /** Wait until every queued write is on disk (tests, and a clean shutdown). */
    fun flush() { io.submit { }.get() }
}
