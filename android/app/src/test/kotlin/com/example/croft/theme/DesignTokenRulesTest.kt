// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * design/CLAUDE.md: never hard-code a colour, a duration, a spring or a radius in Kotlin. This reads the theme and
 * appearance sources and fails on the patterns that would break that rule.
 */
class DesignTokenRulesTest {
    private val roots = listOf("src/main/kotlin/com/example/croft/theme", "src/main/kotlin/com/example/croft/ui/appearance")

    private val forbidden = listOf(
        Regex("""Color\(\s*0x""") to "a colour typed as a number (use a theme role or a token)",
        Regex("""Color\.(Black|White|Red|Green|Blue|Gray|Grey|Yellow|Cyan|Magenta|DarkGray|LightGray)\b""") to "a named colour",
        Regex("""\btween\(""") to "a hard-coded duration (use the motion scheme or CroftMotion)",
        Regex("""durationMillis""") to "a hard-coded duration",
        Regex("""\bspring\(""") to "a hard-coded spring (use CroftMotion)",
        Regex("""RoundedCornerShape\(\s*\d""") to "a hard-coded radius (use MaterialTheme.shapes)",
        Regex("""CornerSize\(\s*\d+\.dp""") to "a hard-coded radius",
    )

    @Test
    fun noColourDurationSpringOrRadiusIsHardCodedInTheThemeAndAppearanceSources() {
        val problems = mutableListOf<String>()
        var files = 0
        for (root in roots) {
            // Timings.kt is the one file that defines waits and fades that tokens.json does not hold yet (T13).
            File(root).walkTopDown().filter { it.isFile && it.extension == "kt" && it.name != "Timings.kt" }.forEach { f ->
                files++
                f.readLines().forEachIndexed { i, line ->
                    val code = line.substringBefore("//")
                    if (code.trimStart().startsWith("*") || code.trimStart().startsWith("/*")) return@forEachIndexed
                    for ((re, why) in forbidden) if (re.containsMatchIn(code)) problems += "${f.name}:${i + 1} $why: ${line.trim()}"
                }
            }
        }
        assertTrue("found only $files source files; is the working directory the app module?", files >= 10)
        assertTrue("design/CLAUDE.md rule broken:\n" + problems.joinToString("\n"), problems.isEmpty())
    }
}
