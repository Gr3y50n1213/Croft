// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import android.content.Context
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/** One line of the open-source licence list: a library, its version and its licence (an SPDX expression as the library declares it). */
data class LicenceEntry(val name: String, val version: String, val licence: String, val origin: Origin) {
    /** Where the library comes from; the list is grouped by it. */
    enum class Origin(val wire: String) { ANDROID("Android"), RUST("Rust"), NATIVE("Native"), FONT("Font"), OTHER("") }
}

/**
 * The list S60 shows under "Open-source licences". It is the checked-in file `assets/licenses/open-source-licences.json`, made by
 * `android/tools/gen_licences.py` from the Rust crates the library is built from (`cargo metadata`), the release runtime classpath
 * (Gradle) and what is compiled in without a crate of its own. CI checks that the file matches the current `Cargo.lock` and
 * `libs.versions.toml`, so it cannot silently go stale.
 */
object Licences {
    /** The asset path. */
    const val ASSET = "licenses/open-source-licences.json"

    /** Entries in [text]; a damaged file gives none (the screen then says the list could not be loaded). */
    fun parse(text: String): List<LicenceEntry> {
        val root = try { Json.parseToJsonElement(text) as? JsonObject } catch (e: Exception) { null } ?: return emptyList()
        val entries = root["entries"] as? JsonArray ?: return emptyList()
        return entries.mapNotNull { element ->
            val o = element as? JsonObject ?: return@mapNotNull null
            fun field(key: String) = (o[key] as? JsonPrimitive)?.contentOrNull
            val name = field("name")?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            val origin = LicenceEntry.Origin.entries.firstOrNull { it.wire == field("origin") } ?: LicenceEntry.Origin.OTHER
            LicenceEntry(name, field("version").orEmpty(), field("licence").orEmpty().ifBlank { "?" }, origin)
        }
    }

    /** Reads the list from the app's assets. */
    fun load(context: Context): List<LicenceEntry> = try {
        context.assets.open(ASSET).use { parse(it.readBytes().toString(Charsets.UTF_8)) }
    } catch (e: java.io.IOException) {
        emptyList()
    }
}
