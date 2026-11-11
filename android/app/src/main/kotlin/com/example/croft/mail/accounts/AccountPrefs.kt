// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/**
 * The colour an account is shown with in S44 and S45. A choice among theme roles (never a typed colour, so it follows the
 * theme and its contrast), kept by name.
 */
enum class AccountColour { PRIMARY, SECONDARY, TERTIARY, AMBER, ERROR, NEUTRAL }

/**
 * What the app itself remembers about its accounts, because the core has no field for it (S44, S45): the order of the list, the
 * name the person gave an account and its colour. Display-only: nothing here reaches a server, and nothing here is an address or a
 * secret. Keyed by the core's account id, which is never reused (a removed account's entries are dropped with [without]).
 */
data class AccountPrefs(
    /** Account ids in the order the person chose; accounts not listed follow, by id. */
    val order: List<AccountId> = emptyList(),
    /** The name the person gave an account; absent means the core's label. */
    val names: Map<AccountId, String> = emptyMap(),
    val colours: Map<AccountId, AccountColour> = emptyMap(),
) {
    /** [accounts] in the person's order. */
    fun ordered(accounts: List<AccountInfo>): List<AccountInfo> {
        val rank = order.withIndex().associate { it.value to it.index }
        return accounts.sortedWith(compareBy({ rank[it.id] ?: Int.MAX_VALUE }, { it.id }))
    }

    /** The name to show for [info] when it is not the sample mailbox: the person's name, else the core's label. */
    fun nameOf(info: AccountInfo): String = names[info.id]?.takeIf { it.isNotBlank() } ?: info.label

    /** [id] moves one place up ([delta] -1) or down (+1) within [shown], the ids as the list shows them now. */
    fun moved(shown: List<AccountId>, id: AccountId, delta: Int): AccountPrefs {
        val from = shown.indexOf(id)
        val to = from + delta
        if (from < 0 || to !in shown.indices) return this
        val next = shown.toMutableList()
        next.removeAt(from)
        next.add(to, id)
        return copy(order = next)
    }

    /** Give [id] this name; a blank name goes back to the core's label. */
    fun withName(id: AccountId, name: String): AccountPrefs {
        val trimmed = name.trim().take(MAX_NAME)
        return copy(names = if (trimmed.isEmpty()) names - id else names + (id to trimmed))
    }

    fun withColour(id: AccountId, colour: AccountColour?): AccountPrefs =
        copy(colours = if (colour == null) colours - id else colours + (id to colour))

    /** Forget everything about a removed account. */
    fun without(id: AccountId): AccountPrefs = AccountPrefs(order - id, names - id, colours - id)

    companion object {
        /** The longest name S45 accepts. */
        const val MAX_NAME = 60
    }
}

/** The stored shape (`croft-accounts.json`): a version, the order, names and colours. A damaged file, or a part that cannot be read, reads as empty. */
object AccountPrefsJson {
    /** Written by this build. */
    const val VERSION = 1

    /** The canonical file text. */
    fun encode(p: AccountPrefs): String = Json { prettyPrint = true; prettyPrintIndent = "  " }.encodeToString(
        JsonObject.serializer(),
        JsonObject(
            linkedMapOf(
                "version" to JsonPrimitive(VERSION),
                "order" to JsonArray(p.order.map { JsonPrimitive(it) }),
                "names" to JsonObject(p.names.entries.sortedBy { it.key }.associate { it.key.toString() to JsonPrimitive(it.value) }),
                "colours" to JsonObject(p.colours.entries.sortedBy { it.key }.associate { it.key.toString() to JsonPrimitive(it.value.name) }),
            ),
        ),
    ) + "\n"

    /** The prefs in [text], or empty ones for whatever cannot be read. */
    fun decodeOrDefault(text: String): AccountPrefs {
        val obj = try { Json.parseToJsonElement(text) as? JsonObject } catch (e: Exception) { null } ?: return AccountPrefs()
        val version = (obj["version"] as? JsonPrimitive)?.longOrNull ?: VERSION.toLong()
        if (version > VERSION) return AccountPrefs()
        val order = (obj["order"] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.longOrNull }?.distinct().orEmpty()
        val names = (obj["names"] as? JsonObject)?.entries?.mapNotNull { (k, v) ->
            val id = k.toLongOrNull() ?: return@mapNotNull null
            val name = (v as? JsonPrimitive)?.contentOrNull?.trim()?.take(AccountPrefs.MAX_NAME)?.takeIf { it.isNotEmpty() } ?: return@mapNotNull null
            id to name
        }?.toMap().orEmpty()
        val colours = (obj["colours"] as? JsonObject)?.entries?.mapNotNull { (k, v) ->
            val id = k.toLongOrNull() ?: return@mapNotNull null
            val colour = AccountColour.entries.firstOrNull { it.name == (v as? JsonPrimitive)?.contentOrNull } ?: return@mapNotNull null
            id to colour
        }?.toMap().orEmpty()
        return AccountPrefs(order, names, colours)
    }
}
