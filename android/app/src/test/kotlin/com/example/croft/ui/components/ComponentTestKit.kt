// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import com.example.croft.CroftTheme
import com.example.croft.haptics.Haptics
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ThemeSettings

/** The settings every component test starts from: Moss, light, and the animation level under test. */
fun settingsAt(level: AnimationLevel = AnimationLevel.FULL): ThemeSettings = ThemeSettings.Default.copy(animation = level)

/** Show [content] under the app theme at [level]; every haptic goes to [haptics]. */
fun ComposeContentTestRule.show(
    level: AnimationLevel = AnimationLevel.FULL,
    haptics: Haptics = RecordingHaptics(),
    content: @Composable () -> Unit,
) {
    setContent {
        CroftTheme(settingsAt(level), systemDark = false) {
            CompositionLocalProvider(LocalHaptics provides haptics) { content() }
        }
    }
}

/** A change the test makes itself, on the UI thread, so a paused clock still sees it (see the T12 notes). */
fun ComposeContentTestRule.ui(block: () -> Unit) {
    runOnUiThread { block(); Snapshot.sendApplyNotifications() }
}

/** The node that exposes the motion probe [name] (searched in the unmerged tree, where every probe is). */
fun ComposeContentTestRule.probeNode(name: String): SemanticsNodeInteraction =
    onNode(SemanticsMatcher("has probe $name") { it.config.getOrNull(MotionProbes)?.containsKey(name) == true }, useUnmergedTree = true)

/** The current value of probe [name]. */
fun ComposeContentTestRule.probe(name: String): Float =
    probeNode(name).fetchSemanticsNode().config[MotionProbes].getValue(name)()

/** Nodes a person can act on: a click action, a toggle or a selection. */
val isInteractive: SemanticsMatcher = SemanticsMatcher("is interactive") {
    val c = it.config
    c.contains(androidx.compose.ui.semantics.SemanticsActions.OnClick) ||
        c.contains(SemanticsProperties.ToggleableState) ||
        c.contains(SemanticsProperties.Selected)
}
