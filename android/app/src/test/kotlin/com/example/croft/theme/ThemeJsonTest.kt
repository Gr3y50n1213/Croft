// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftPreset
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The stored shape, `croft-theme.json` export and import, and the fallback for a file that cannot be read. */
class ThemeJsonTest {
    private val everything = ThemeSettings(
        mode = ModePreference.DARK, pureBlack = true, source = ColourSource.CUSTOM, preset = CroftPreset.TIDE,
        seed = 0xFF8A4FFF, style = ThemeStyle.EXPRESSIVE, contrast = CroftContrast.HIGH,
        density = DensityLevel.COMPACT, roundness = Roundness.ROUND, listStyle = ListStyle.FLAT,
        avatars = AvatarStyle.HIDDEN, unreadEmphasis = UnreadEmphasis.DOT, textScale = 1.35f,
        uiFont = UiFont.ATKINSON, readingFont = ReadingFont.SERIF, animation = AnimationLevel.REDUCED,
        haptics = HapticsLevel.FULL,
        accountColors = mapOf("acct-2" to 0xFF2F6F73, "acct-1" to 0xFF4F6F52),
    )

    private fun imported(text: String): ThemeSettings =
        (ThemeJson.decode(text) as ThemeImportResult.Imported).settings

    private fun problem(text: String): ThemeImportProblem =
        (ThemeJson.decode(text) as ThemeImportResult.Rejected).problem

    @Test
    fun exportThenImportThenExportGivesTheIdenticalFile() {
        for (s in listOf(ThemeSettings.Default, everything)) {
            val file = ThemeJson.encode(s)
            val back = imported(file)
            assertEquals(s, back)
            assertEquals(file, ThemeJson.encode(back))
        }
    }

    @Test
    fun everyPresetAndTextSizeSurvivesTheFile() {
        for (p in CroftPreset.entries) {
            val s = ThemeSettings.Default.withPreset(p)
            assertEquals(ThemeJson.encode(s), ThemeJson.encode(imported(ThemeJson.encode(s))))
        }
        var pct = 85
        while (pct <= 150) {
            val s = ThemeSettings.Default.copy(textScale = ThemeSettings.clampTextScale(pct / 100f))
            assertEquals("text size $pct", s, imported(ThemeJson.encode(s)))
            pct += 5
        }
    }

    @Test
    fun theFileHasTheStoredShapeOfTheDesignDocs() {
        val text = ThemeJson.encode(ThemeSettings.Default)
        // Same keys, same order, same values as the example in docs/ux/theme.md, plus the leading version.
        assertEquals(
            """
            {
              "version": 1,
              "mode": "system",
              "pureBlack": false,
              "source": "preset",
              "preset": "moss",
              "seed": "#4F6F52",
              "style": "tonalSpot",
              "contrast": "standard",
              "density": "default",
              "roundness": "standard",
              "listStyle": "container",
              "avatars": "letters",
              "unreadEmphasis": "boldDot",
              "textScale": 1.0,
              "uiFont": "system",
              "readingFont": "system",
              "animation": "system",
              "haptics": "subtle",
              "accountColors": {}
            }
            """.trimIndent() + "\n",
            text,
        )
    }

    @Test
    fun theExampleInTheDesignDocsLoadsAsMossStandardSystem() {
        val doc = """
            {"mode":"system","pureBlack":false,"source":"preset","preset":"moss","seed":"#4F6F52","style":"tonalSpot",
             "contrast":"standard","density":"default","roundness":"standard","listStyle":"container","avatars":"letters",
             "unreadEmphasis":"boldDot","textScale":1.0,"uiFont":"system","readingFont":"system","animation":"system",
             "haptics":"subtle","accountColors":{"acct-1":"#4F6F52"}}
        """.trimIndent()
        val s = imported(doc)
        assertEquals(ThemeSettings.Default.copy(accountColors = mapOf("acct-1" to 0xFF4F6F52)), s)
    }

    @Test
    fun missingKeysKeepTheDefaultAndUnknownKeysAreIgnored() {
        val s = imported("""{"version":1,"preset":"wax","somethingNew":{"a":1}}""")
        assertEquals(CroftPreset.WAX, s.preset)
        assertEquals(CroftPreset.WAX.seed, s.seed)
        assertEquals(ThemeSettings.Default.density, s.density)
        assertEquals(ThemeSettings.Default, imported("{}"))
    }

    @Test
    fun anEmptyFileIsRejectedAndNamedAsEmpty() {
        assertEquals(ThemeImportProblem.EMPTY, problem(""))
        assertEquals(ThemeImportProblem.EMPTY, problem("  \n "))
    }

    @Test
    fun aCorruptFileIsRejected() {
        assertEquals(ThemeImportProblem.NOT_JSON, problem("{not json"))
        assertEquals(ThemeImportProblem.NOT_JSON, problem("\u0000\u0001\u0002"))
        assertEquals(ThemeImportProblem.NOT_JSON, problem("""{"mode": "dark""""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("[1,2,3]"))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("42"))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"mode":"sepia"}"""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"mode":3}"""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"seed":"red"}"""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"textScale":"big"}"""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"pureBlack":"yes"}"""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"accountColors":{"a":"nope"}}"""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"version":"one"}"""))
        assertEquals(ThemeImportProblem.NOT_A_THEME, problem("""{"version":0}"""))
        assertEquals(ThemeImportProblem.TOO_BIG, problem("{" + " ".repeat(ThemeJson.MAX_BYTES) + "}"))
    }

    @Test
    fun aFileFromANewerCroftIsRejected() {
        assertEquals(ThemeImportProblem.TOO_NEW, problem("""{"version":2,"mode":"dark"}"""))
        assertEquals(ThemeImportProblem.TOO_NEW, problem("""{"version":99}"""))
    }

    @Test
    fun textSizeOutsideTheRangeIsClamped() {
        assertEquals(1.5f, imported("""{"textScale":3.0}""").textScale)
        assertEquals(0.85f, imported("""{"textScale":0.1}""").textScale)
        assertTrue(ThemeSettings.clampTextScale(1.234f) in 0.85f..1.5f)
    }

    @Test
    fun theStoredFileFallsBackToMossStandardSystemForAnythingUnreadable() {
        for (bad in listOf("", "garbage", "{\"version\":7}", "{\"mode\":\"sepia\"}", "null", "[]")) {
            assertEquals("for <$bad>", ThemeSettings.Default, ThemeJson.decodeOrDefault(bad))
        }
        // The serializer used by DataStore does the same with bytes.
        runBlocking {
            for (bad in listOf("", "garbage", "{\"version\":7}")) {
                assertEquals(ThemeSettings.Default, ThemeSettingsSerializer.readFrom(ByteArrayInputStream(bad.toByteArray())))
            }
            assertEquals(ThemeSettings.Default, ThemeSettingsSerializer.readFrom(ByteArrayInputStream(byteArrayOf(0xFF.toByte(), 0xFE.toByte(), 0x00))))
            val out = ByteArrayOutputStream()
            ThemeSettingsSerializer.writeTo(everything, out)
            assertEquals(everything, ThemeSettingsSerializer.readFrom(ByteArrayInputStream(out.toByteArray())))
        }
    }

    @Test
    fun theStoreWritesAndReadsBackThroughDataStoreAndSurvivesACorruptFile() {
        val dir = createTempDirectory("croft-theme-test").toFile()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        try {
            val file = File(dir, "croft-theme.json")
            file.writeText("{{{ this is not json")
            val store = ThemeStore.onFile(file, scope)
            runBlocking {
                assertEquals(ThemeSettings.Default, store.settings.first())
                store.set(everything)
                assertEquals(everything, store.settings.first())
                store.update { it.copy(density = DensityLevel.COMFORTABLE) }
                assertEquals(DensityLevel.COMFORTABLE, store.settings.first().density)
            }
            // What is on disk is the same canonical file an export writes.
            assertEquals(ThemeJson.encode(everything.copy(density = DensityLevel.COMFORTABLE)), file.readText())
            assertFalse(file.readText().contains("password"))
        } finally {
            scope.cancel()
            dir.deleteRecursively()
        }
    }

    @Test
    fun hexParsingIsStrict() {
        assertEquals(0xFF4F6F52, ThemeJson.parseHex("#4f6f52"))
        assertEquals(0xFF4F6F52, ThemeJson.parseHex(" 4F6F52 "))
        assertEquals(null, ThemeJson.parseHex("#4F6F5"))
        assertEquals(null, ThemeJson.parseHex("#GGGGGG"))
        assertEquals(null, ThemeJson.parseHex(""))
        assertEquals("#00FF7F", ThemeJson.hex(0xFF00FF7F))
    }

    @Test
    fun theSettingsClassHoldsNoSecretFields() {
        // The theme file is shared and backed up as plain text; keep it free of anything account-private.
        val names = ThemeSettings::class.java.declaredFields.map { it.name.lowercase() }
        assertTrue(names.none { "password" in it || "token" in it || "secret" in it || "email" in it })
    }
}
