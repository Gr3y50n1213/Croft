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
 * The gallery is reachable only in debug builds: its code is in the `debug` source set (a release build does not contain it),
 * the release source set has a stub with the same API that says it is not there, and nothing in `main` can open it by a hidden
 * gesture. These read the source tree (the working directory is the app module).
 */
class GalleryIsDebugOnlyTest {
    private fun kotlinFiles(dir: String) = File(dir).walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()

    @Test
    fun theGalleryCodeIsInTheDebugSourceSetOnly() {
        assertTrue(File("src/debug/kotlin/com/example/croft/debug/gallery/GalleryCatalog.kt").isFile)
        assertTrue(File("src/debug/kotlin/com/example/croft/debug/gallery/GalleryScreen.kt").isFile)
        val inMain = kotlinFiles("src/main/kotlin").filter { it.readText().contains("debug.gallery") || it.name.startsWith("Gallery") }
        assertTrue("gallery code in main: $inMain", inMain.isEmpty())
        assertTrue("no gallery code in release", kotlinFiles("src/release/kotlin").none { it.readText().contains("GalleryCatalog") })
    }

    @Test
    fun theReleaseStubHasTheSameApiAndSaysItIsNotAvailable() {
        val stub = File("src/release/kotlin/com/example/croft/debug/DebugGallery.kt").readText()
        assertTrue(stub, Regex("""const val available: Boolean = false""").containsMatchIn(stub))
        assertTrue(stub.contains("fun Screen(onBack: () -> Unit)"))
        val real = File("src/debug/kotlin/com/example/croft/debug/DebugGallery.kt").readText()
        assertTrue(real.contains("const val available: Boolean = true"))
        assertTrue(real.contains("fun Screen(onBack: () -> Unit)"))
    }

    @Test
    fun mainReachesTheGalleryOnlyThroughTheDebugGalleryApi_behindTheAvailableFlag() {
        val users = kotlinFiles("src/main/kotlin").filter { it.readText().contains("DebugGallery") }
        assertEquals(listOf("AppNavigation.kt"), users.map { it.name })
        val text = users.single().readText()
        assertTrue("the settings entry is only offered when available", text.contains("if (DebugGallery.available)"))
        assertTrue("a saved route cannot reach it in a release build", text.contains("route == Route.GALLERY && !DebugGallery.available"))
    }

    @Test
    fun noHiddenGestureOpensItAndNoActivityIsDeclaredForIt() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        assertFalse(manifest.contains("Gallery"))
        for (f in kotlinFiles("src/main/kotlin")) {
            val t = f.readText()
            assertFalse("${f.name} must not open the gallery on a secret tap", Regex("""(?i)(tapCount|clickCount|taps\s*>=)[^\n]*gallery""").containsMatchIn(t))
        }
        assertFalse(File("src/main/AndroidManifest.xml").readText().contains("android:debuggable"))
    }
}
