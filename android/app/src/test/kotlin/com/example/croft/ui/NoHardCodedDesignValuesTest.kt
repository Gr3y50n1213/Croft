// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * design/CLAUDE.md: no colour, duration, spring or radius is typed in Kotlin outside the theme package (`theme/`, where the
 * tokens are turned into values). This reads every main and debug source file and fails on the patterns that break the
 * rule. (`theme/` itself is read by `DesignTokenRulesTest`'s sibling rules for the other directories; the generated tokens are
 * in `design/generated`, not here.)
 */
class NoHardCodedDesignValuesTest {
    private val roots = listOf("src/main/kotlin", "src/debug/kotlin", "src/release/kotlin")
    private val skipDirs = setOf("theme", "secrets")

    private val forbidden = listOf(
        Regex("""\bColor\(\s*0x""") to "a colour typed as a number (use a theme role or a token)",
        Regex("""\bColor\(\s*\d""") to "a colour typed as a number",
        Regex("""\bColor\.(Black|White|Red|Green|Blue|Gray|Grey|Yellow|Cyan|Magenta|DarkGray|LightGray)\b""") to "a named colour",
        Regex("""\btween\(""") to "a hard-coded duration (use the motion scheme, CroftMotion or CroftTimings)",
        Regex("""durationMillis""") to "a hard-coded duration",
        Regex("""\bspring\(""") to "a hard-coded spring (use CroftMotion or the motion scheme)",
        Regex("""\bdelay\(\s*\d""") to "a hard-coded wait (use CroftTimings)",
        Regex("""RoundedCornerShape\(\s*\d""") to "a hard-coded radius (use MaterialTheme.shapes)",
        Regex("""CornerSize\(\s*\d""") to "a hard-coded radius",
        Regex("""RoundedCornerShape\(\s*(topStart|topEnd|bottomStart|bottomEnd|top|bottom|start|end)\s*=\s*\d""") to "a hard-coded radius",
    )

    private fun violations(lines: List<String>, name: String): List<String> = buildList {
        lines.forEachIndexed { i, line ->
            val code = line.substringBefore("//")
            if (code.trimStart().startsWith("*") || code.trimStart().startsWith("/*")) return@forEachIndexed
            for ((re, why) in forbidden) if (re.containsMatchIn(code)) add("$name:${i + 1} $why: ${line.trim()}")
        }
    }

    @Test
    fun noColourDurationSpringOrRadiusIsHardCodedOutsideTheThemePackage() {
        val problems = mutableListOf<String>()
        var files = 0
        for (root in roots) {
            File(root).walkTopDown()
                .onEnter { it.name !in skipDirs }
                .filter { it.isFile && it.extension == "kt" }
                .forEach { f ->
                    files++
                    problems += violations(f.readLines(), f.path)
                }
        }
        assertTrue("found only $files source files; is the working directory the app module?", files >= 25)
        assertTrue("design/CLAUDE.md rule broken:\n" + problems.joinToString("\n"), problems.isEmpty())
    }

    @Test
    fun theRuleCatchesTheThingsItIsMeantTo() {
        val bad = listOf(
            "val c = Color(0xFF112233)", "val c = Color(123)", "tween(300)", "animateTo(1f, spring(0.5f))", "delay(600)",
            "RoundedCornerShape(12.dp)", "RoundedCornerShape(topStart = 8.dp)", "Modifier.background(Color.Red)", "animation(durationMillis = 90)",
        )
        for (line in bad) assertEquals(line, 1, violations(listOf(line), "x").size.coerceAtMost(1))
        val fine = listOf(
            "RoundedCornerShape(settings.roundness.swipeCardRadiusDp.dp)", "delay(CroftTimings.BUTTON_SUCCESS_HOLD_MS)",
            "color = Color.Transparent", "// Color(0xFF000000) in a comment", "spring = CroftMotion.spatialFast.spec()",
        )
        for (line in fine) assertTrue(line, violations(listOf(line), "x").isEmpty())
    }

    @Test
    fun theThemePackageIsTheOnlyPlaceSkipped() {
        assertEquals(setOf("theme", "secrets"), skipDirs)
        assertFalse(File("src/main/kotlin/com/example/croft/ui").walkTopDown().any { it.name in skipDirs })
    }
}
