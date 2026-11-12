// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.search

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.search.InMemorySearchHistory
import com.example.croft.mail.search.SearchController
import com.example.croft.mail.search.ServerSearchResult
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.ScreenshotKit.Device
import java.time.ZoneOffset
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** S27 (recent searches) and S28 (highlighted results, empty, offline). */
enum class SearchScreenId(val label: String) { S27("S27"), S28("S28"), S28_EMPTY("S28-empty"), S28_OFFLINE("S28-offline") }

@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class SearchScreenshotTest(
    private val screen: SearchScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in SearchScreenId.entries) {
                for (device in listOf(Device.COMPACT, Device.EXPANDED)) for (dark in listOf(false, true)) for (font in listOf(1.0f, 2.0f)) add(arrayOf<Any>(screen, device, dark, font, false))
                add(arrayOf<Any>(screen, Device.COMPACT, false, 1.0f, true))
            }
        }
    }

    @get:Rule
    val rule = createComposeRule()

    private fun name() = "${screen.label}-${device.name.lowercase()}-${if (dark) "dark" else "light"}-font${"%.1f".format(fontScale)}-${if (rtl) "rtl" else "ltr"}"

    @Test
    fun screenshot() {
        ScreenshotKit.configure(device, rtl, fontScale)
        val settings = ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT, animation = AnimationLevel.OFF)
        val backend = when (screen) {
            SearchScreenId.S28_OFFLINE -> FakeSearchBackend(server = ServerSearchResult(emptyList(), offline = true))
            else -> FakeSearchBackend(bodiesPartial = true, server = ServerSearchResult(emptyList()))
        }
        val controller = SearchController(backend, CoroutineScope(SupervisorJob() + Dispatchers.Main), InMemorySearchHistory(listOf("lunch", "from:mira", "has:attachment invoice")), debounceMs = 0)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                    SearchScreen(controller, searchAccounts, onOpenMessage = {}, onBack = {}, zone = ZoneOffset.UTC)
                }
            }
        }
        when (screen) {
            SearchScreenId.S27 -> Unit
            SearchScreenId.S28, SearchScreenId.S28_OFFLINE -> {
                controller.submit("lunch")
                rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(SearchTags.result(1))).fetchSemanticsNodes().isNotEmpty() }
            }
            SearchScreenId.S28_EMPTY -> {
                controller.submit("nothing like this")
                rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(SearchTags.EMPTY)).fetchSemanticsNodes().isNotEmpty() }
            }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), name())
    }
}
