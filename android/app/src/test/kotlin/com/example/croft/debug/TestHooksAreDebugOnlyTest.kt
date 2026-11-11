// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The test hooks (the way a test swaps the app's secrets for an in-memory wrapping key) are reachable only in debug builds, as the
 * component gallery is (`GalleryIsDebugOnlyTest`): the real object is in the `debug` source set, the release source set has a stub with
 * the same API that always reads as null and keeps nothing, and `main` has no hook of its own. These read the source tree (the working
 * directory is the app module). `tools/check-release-outputs.sh` also looks for the debug-only field in the release build's R8 mapping.
 */
class TestHooksAreDebugOnlyTest {
    private fun kotlinFiles(dir: String) = File(dir).walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()

    @Test
    fun theHookIsInTheDebugSourceSetAndTheReleaseStubKeepsNothing() {
        val real = File("src/debug/kotlin/com/example/croft/debug/TestHooks.kt").readText()
        assertTrue(real.contains("@Volatile var debugOnlySecretsFactory"))
        val stub = File("src/release/kotlin/com/example/croft/debug/TestHooks.kt").readText()
        assertTrue("the stub reads as null", Regex("""get\(\)\s*=\s*null""").containsMatchIn(stub))
        assertTrue("the stub ignores what it is given", Regex("""set\(value\)\s*=\s*Unit""").containsMatchIn(stub))
        assertFalse("the stub holds nothing", stub.contains("debugOnlySecretsFactory") || stub.contains("@Volatile"))
        assertTrue("same API", stub.contains("var secretsFactory: ((Context, File) -> AppSecrets)?"))
    }

    @Test
    fun mainHasNoTestHookOfItsOwnAndReachesTheHookOnlyThroughTestHooks() {
        val main = kotlinFiles("src/main/kotlin")
        for (f in main) {
            val t = f.readText()
            assertFalse("${f.name} must not define the hook", Regex("""(var|val)\s+secretsFactory""").containsMatchIn(t))
            assertFalse("${f.name} must not carry a reset for tests", t.contains("resetForTests"))
            assertFalse("${f.name} must not name the debug-only field", t.contains("debugOnlySecretsFactory"))
        }
        val users = main.filter { it.readText().contains("TestHooks") }
        assertEquals(listOf("MailRuntime.kt"), users.map { it.name })
        assertTrue(users.single().readText().contains("TestHooks.secretsFactory?.invoke("))
    }

    @Test
    fun theReleaseCheckLooksForTheDebugOnlyFieldInTheMapping() {
        val script = File("../tools/check-release-outputs.sh").readText()
        assertTrue(script.contains("debugOnlySecretsFactory") && script.contains("mapping.txt"))
    }
}
