// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.compose.ComposeFill
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.EncryptionDecision
import com.example.croft.mail.compose.RequiredKeyMissingPolicy
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.ScreenshotKit.Device
import kotlinx.coroutines.runBlocking
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.croft_ffi.OutboxItemState
import uniffi.croft_ffi.OutboxReason

/** The pictures of T18d: S23 (a reply being written, and a message with an error chip, Cc and an attachment), S24 with the fake policy, S26. */
enum class ComposeScreenId(val label: String) { S23("S23"), S23_FILLED("S23-filled"), S24("S24"), S26("S26") }

/**
 * Screenshot tests (Roborazzi, JVM) of S23, S24 and S26: compact and expanded width, light and dark, font scale 1.0 and 2.0 (all
 * left-to-right), plus one right-to-left picture of each at compact width, light, font 1.0. 4 screens x 2 x 2 x 2 = 32, and 4 for
 * right-to-left. Names carry the screen ID: `S24-compact-dark-font2.0-ltr`. Pictures go to `app/build/outputs/screenshots/`; none is
 * committed as a golden (record some on CI, see `ScreenshotKit.Golden`). S24 is the content of the sheet on the screen's own background:
 * a sheet is a window of its own, which the picture of the root would not hold.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class ComposeScreenshotTest(
    private val screen: ComposeScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in ComposeScreenId.entries) {
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
        when (screen) {
            ComposeScreenId.S23 -> compose(settings, ComposeFill(to = listOf("mira@example.org"), subject = "Re: Server migration notes", body = "Thanks, tonight works.\n\nSam"), withAttachment = false)
            ComposeScreenId.S23_FILLED -> compose(
                settings,
                ComposeFill(to = listOf("mira@example.org", "dan@oldmail"), cc = listOf("ops@example.org"), subject = "Photos from the weekend", body = "Here they are."),
                withAttachment = true,
            )
            ComposeScreenId.S24 -> {
                // The fake policy: encryption is required and Dan has no key.
                val missing = (runBlocking { RequiredKeyMissingPolicy(listOf("dan@oldmail.org")).check(1, listOf("mira@example.org", "dan@oldmail.org")) } as EncryptionDecision.RequiredKeysMissing).missing
                rule.setContent {
                    CroftTheme(settings, systemDark = dark) {
                        CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                            Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surfaceContainerLow).padding(16.dp), contentAlignment = Alignment.BottomCenter) {
                                Box(Modifier.widthIn(max = 640.dp)) { EncryptionCheckContent(missing, {}, {}) }
                            }
                        }
                    }
                }
                rule.waitForIdle()
            }
            ComposeScreenId.S26 -> {
                val f = ComposeFixture()
                f.backend.addOutboxItem(1, outboxItem(1u, OutboxItemState.QUEUED, OutboxReason.TRYING_AGAIN, subject = "Meeting notes"))
                f.backend.addOutboxItem(1, outboxItem(2u, OutboxItemState.SENDING, subject = "Photos from the weekend"))
                f.backend.addOutboxItem(1, outboxItem(3u, OutboxItemState.FAILED, OutboxReason.MESSAGE_REFUSED, subject = "Invoice 4471 reply"))
                rule.setContent {
                    CroftTheme(settings, systemDark = dark) { CompositionLocalProvider(LocalHaptics provides NoHaptics) { OutboxHost(f.session, onBack = {}, onEdit = {}) } }
                }
                rule.waitUntil(20_000) { rule.onAllNodes(hasText("Invoice 4471 reply")).fetchSemanticsNodes().isNotEmpty() }
            }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), name())
    }

    private fun compose(settings: ThemeSettings, fill: ComposeFill, withAttachment: Boolean) {
        val f = ComposeFixture()
        val start: ComposeStart = if (withAttachment) {
            // A draft that already holds a 30 MB file (over the 25 MB at which Send is disabled), so the chip shows its warning.
            val file = java.io.File.createTempFile("croft-shot", ".bin").also { it.deleteOnExit() }
            file.writeBytes(ByteArray(30 * 1024 * 1024))
            runBlocking {
                val content = uniffi.croft_ffi.DraftContent(null, fill.to.map { uniffi.croft_ffi.FfiAddress(null, it) }, fill.cc.map { uniffi.croft_ffi.FfiAddress(null, it) }, emptyList(), fill.subject, fill.body, null, emptyList())
                val saved = f.backend.saveDraft(1, null, content)
                f.backend.addAttachmentFile(1, saved.id, file.path, "weekend.zip", "application/zip")
                ComposeStart.Draft(1, saved.id)
            }
        } else ComposeStart.Fill(fill, account = 1)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) { ComposeHost(f.session, start, {}, tools = UiTestAddressTools) }
            }
        }
        rule.waitUntil(20_000) { rule.onAllNodes(hasTestTag(ComposeTags.SUBJECT)).fetchSemanticsNodes().isNotEmpty() }
        if (withAttachment) rule.waitUntil(20_000) { rule.onAllNodes(hasTestTag(ComposeTags.attachment(0))).fetchSemanticsNodes().isNotEmpty() }
        if (withAttachment) rule.waitUntil(20_000) { f.backend.calls.contains("checkDraft") }
    }
}
