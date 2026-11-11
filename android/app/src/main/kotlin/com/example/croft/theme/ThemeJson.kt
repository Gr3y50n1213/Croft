// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftPreset
import java.util.Locale
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull

/** Why a theme file was not applied. The screens show a plain sentence for each (strings `appearance_import_*`). */
enum class ThemeImportProblem {
    /** Nothing in the file. */
    EMPTY,
    /** The file is larger than a theme file can be. */
    TOO_BIG,
    /** Not JSON. */
    NOT_JSON,
    /** JSON, but not a Croft theme (not an object, or a known setting with a value Croft does not have). */
    NOT_A_THEME,
    /** Written by a newer Croft than this one. */
    TOO_NEW,
}

/** The outcome of reading a theme file: the settings, or the reason it was refused. Nothing is applied on a refusal. */
sealed interface ThemeImportResult {
    data class Imported(val settings: ThemeSettings) : ThemeImportResult
    data class Rejected(val problem: ThemeImportProblem) : ThemeImportResult
}

/**
 * The `croft-theme.json` format: the stored shape of docs/ux/theme.md plus a leading `version`. Writing is canonical
 * (fixed key order, two-space indent, one trailing newline), so export, import and export again gives the same bytes.
 * Reading is strict about values (an unknown option refuses the file) and tolerant about keys (a missing key keeps the
 * default, an unknown key is ignored), so a file from an older Croft still loads.
 */
object ThemeJson {
    /** Version written by this build. A file with a higher version is refused; a file with none counts as version 1. */
    const val VERSION = 1

    /** Largest file accepted. A real theme file is under 1 KB. */
    const val MAX_BYTES = 64 * 1024

    const val FILE_NAME = "croft-theme.json"
    const val MIME_TYPE = "application/json"

    private val parser = Json

    /** The canonical file text for [s]. */
    fun encode(s: ThemeSettings): String {
        val accounts = JsonObject(
            s.accountColors.toSortedMap().mapValues<String, Long, JsonElement> { JsonPrimitive(hex(it.value)) },
        )
        val obj = JsonObject(
            linkedMapOf(
                "version" to JsonPrimitive(VERSION),
                "mode" to JsonPrimitive(s.mode.id),
                "pureBlack" to JsonPrimitive(s.pureBlack),
                "source" to JsonPrimitive(s.source.id),
                "preset" to JsonPrimitive(s.preset.id),
                "seed" to JsonPrimitive(hex(s.seed)),
                "style" to JsonPrimitive(s.style.id),
                "contrast" to JsonPrimitive(s.contrast.name.lowercase(Locale.ROOT)),
                "density" to JsonPrimitive(s.density.id),
                "roundness" to JsonPrimitive(s.roundness.id),
                "listStyle" to JsonPrimitive(s.listStyle.id),
                "avatars" to JsonPrimitive(s.avatars.id),
                "unreadEmphasis" to JsonPrimitive(s.unreadEmphasis.id),
                "textScale" to JsonPrimitive(ThemeSettings.clampTextScale(s.textScale).toDouble()),
                "uiFont" to JsonPrimitive(s.uiFont.id),
                "readingFont" to JsonPrimitive(s.readingFont.id),
                "animation" to JsonPrimitive(s.animation.id),
                "haptics" to JsonPrimitive(s.haptics.id),
                "accountColors" to accounts,
            ),
        )
        return pretty(obj, 0) + "\n"
    }

    /** Read a file's text. Never throws. */
    fun decode(text: String): ThemeImportResult {
        if (text.isBlank()) return ThemeImportResult.Rejected(ThemeImportProblem.EMPTY)
        if (text.length > MAX_BYTES) return ThemeImportResult.Rejected(ThemeImportProblem.TOO_BIG)
        val root = try {
            parser.parseToJsonElement(text)
        } catch (e: Exception) {
            return ThemeImportResult.Rejected(ThemeImportProblem.NOT_JSON)
        }
        if (root !is JsonObject) return ThemeImportResult.Rejected(ThemeImportProblem.NOT_A_THEME)
        val version = root["version"]
        if (version != null) {
            val v = (version as? JsonPrimitive)?.takeIf { !it.isString }?.doubleOrNull
                ?: return ThemeImportResult.Rejected(ThemeImportProblem.NOT_A_THEME)
            if (v != Math.floor(v) || v < 1) return ThemeImportResult.Rejected(ThemeImportProblem.NOT_A_THEME)
            if (v > VERSION) return ThemeImportResult.Rejected(ThemeImportProblem.TOO_NEW)
        }
        return try {
            ThemeImportResult.Imported(read(root))
        } catch (e: BadValue) {
            ThemeImportResult.Rejected(ThemeImportProblem.NOT_A_THEME)
        }
    }

    /** For the stored file: the settings, or Moss, Standard, System when the text cannot be read for any reason. */
    fun decodeOrDefault(text: String): ThemeSettings =
        (decode(text) as? ThemeImportResult.Imported)?.settings ?: ThemeSettings.Default

    private class BadValue : Exception()

    private fun read(o: JsonObject): ThemeSettings {
        val d = ThemeSettings.Default
        val preset = option(o, "preset", CroftPreset.entries, d.preset) { it.id }
        val source = option(o, "source", ColourSource.entries, d.source) { it.id }
        val seed = o["seed"]?.let { parseHex(str(it)) ?: throw BadValue() }
            ?: if (source == ColourSource.PRESET) preset.seed else d.seed
        return ThemeSettings(
            mode = option(o, "mode", ModePreference.entries, d.mode) { it.id },
            pureBlack = o["pureBlack"]?.let { bool(it) } ?: d.pureBlack,
            source = source,
            preset = preset,
            seed = seed,
            style = option(o, "style", ThemeStyle.entries, d.style) { it.id },
            contrast = option(o, "contrast", CroftContrast.entries, d.contrast) { it.name.lowercase(Locale.ROOT) },
            density = option(o, "density", DensityLevel.entries, d.density) { it.id },
            roundness = option(o, "roundness", Roundness.entries, d.roundness) { it.id },
            listStyle = option(o, "listStyle", ListStyle.entries, d.listStyle) { it.id },
            avatars = option(o, "avatars", AvatarStyle.entries, d.avatars) { it.id },
            unreadEmphasis = option(o, "unreadEmphasis", UnreadEmphasis.entries, d.unreadEmphasis) { it.id },
            textScale = o["textScale"]?.let { number(it) }?.let { ThemeSettings.clampTextScale(it.toFloat()) } ?: d.textScale,
            uiFont = option(o, "uiFont", UiFont.entries, d.uiFont) { it.id },
            readingFont = option(o, "readingFont", ReadingFont.entries, d.readingFont) { it.id },
            animation = option(o, "animation", AnimationLevel.entries, d.animation) { it.id },
            haptics = option(o, "haptics", HapticsLevel.entries, d.haptics) { it.id },
            accountColors = o["accountColors"]?.let { accounts(it) } ?: d.accountColors,
        )
    }

    private fun <E> option(o: JsonObject, key: String, all: List<E>, default: E, id: (E) -> String): E {
        val v = o[key] ?: return default
        val text = str(v)
        return all.firstOrNull { id(it) == text } ?: throw BadValue()
    }

    private fun str(e: JsonElement): String {
        val p = e as? JsonPrimitive ?: throw BadValue()
        if (p is JsonNull || !p.isString) throw BadValue()
        return p.content
    }

    private fun bool(e: JsonElement): Boolean = (e as? JsonPrimitive)?.booleanOrNull ?: throw BadValue()

    private fun number(e: JsonElement): Double {
        val p = e as? JsonPrimitive ?: throw BadValue()
        if (p.isString) throw BadValue()
        return p.doubleOrNull?.takeIf { it.isFinite() } ?: throw BadValue()
    }

    private fun accounts(e: JsonElement): Map<String, Long> {
        val obj = e as? JsonObject ?: throw BadValue()
        if (obj.size > MAX_ACCOUNTS) throw BadValue()
        return obj.entries.associate { (k, v) -> k to (parseHex(str(v)) ?: throw BadValue()) }
    }

    private const val MAX_ACCOUNTS = 200

    /** `#RRGGBB` for an opaque ARGB value. */
    fun hex(argb: Long): String = String.format(Locale.ROOT, "#%06X", argb and 0xFFFFFFL)

    /** `#RRGGBB` or `RRGGBB` (any case) to an opaque ARGB value, or null. */
    fun parseHex(text: String): Long? {
        val t = text.trim().removePrefix("#")
        if (t.length != 6 || !t.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) return null
        return ThemeSettings.OPAQUE or t.toLong(16)
    }

    private fun pretty(e: JsonElement, depth: Int): String = when (e) {
        is JsonObject ->
            if (e.isEmpty()) "{}" else {
                val pad = "  ".repeat(depth + 1)
                e.entries.joinToString(",\n", "{\n", "\n" + "  ".repeat(depth) + "}") { (k, v) ->
                    pad + JsonPrimitive(k).toString() + ": " + pretty(v, depth + 1)
                }
            }
        else -> e.toString()
    }
}
