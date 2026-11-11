// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.FolderRole
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeDirection
import com.example.croft.swipe.SwipePreset
import com.example.croft.swipe.SwipeSensitivity
import com.example.croft.swipe.SwipeSettings
import com.example.croft.swipe.SwipeSlot
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.swipePillColours
import com.example.croft.ui.appearance.ChoiceRow
import com.example.croft.ui.appearance.SectionLabel
import com.example.croft.ui.appearance.SettingsFrame
import com.example.croft.ui.components.EncryptionMark
import com.example.croft.ui.components.MessageRow
import com.example.croft.ui.components.MessageRowState
import com.example.croft.ui.components.sharedAxisX

/** Test tags of S47 and S48. */
object SwipeTags {
    const val S47 = "S47"
    const val S48 = "S48"
    const val PREVIEW = "S47-preview"
    const val RESET = "S47-reset"
    fun picker(direction: SwipeDirection) = "S47-pick-${direction.name.lowercase()}"
    fun sensitivity(s: SwipeSensitivity) = "S47-sensitivity-${s.id}"
    fun preset(p: SwipePreset) = "S47-preset-${p.id}"
    fun option(a: SwipeAction) = "S48-option-${a.id}"
}

/** The two screens of the swipe settings. */
enum class SwipeRoute { S47, S48_START_TO_END, S48_END_TO_START }

/**
 * S47 with its picker S48: Back from S48 returns to S47 first, the drill-down is the shared axis X at the animation level in force.
 * Stateless about the settings: the caller keeps them (DataStore in the app, a variable in tests).
 */
@Composable
fun SwipeSettingsFlow(
    settings: SwipeSettings,
    onChange: (SwipeSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    startAt: SwipeRoute = SwipeRoute.S47,
    /** Play the looped demonstration in the preview (tests and screenshots switch it off: it never lets a test go idle). */
    demo: Boolean = true,
) {
    var route by rememberSaveable { androidx.compose.runtime.mutableStateOf(startAt) }
    val level = LocalMotionLevel.current
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    BackHandler(enabled = route != SwipeRoute.S47) { route = SwipeRoute.S47 }
    AnimatedContent(
        targetState = route,
        modifier = modifier,
        label = "swipe-route",
        transitionSpec = { sharedAxisX(level, forward = targetState != SwipeRoute.S47, rtl = rtl) },
    ) { current ->
        when (current) {
            SwipeRoute.S47 -> SwipeActionsScreen(
                settings, onChange, onBack,
                demo = demo,
                onPick = { d -> route = if (d == SwipeDirection.START_TO_END) SwipeRoute.S48_START_TO_END else SwipeRoute.S48_END_TO_START },
            )
            SwipeRoute.S48_START_TO_END -> ChooseSwipeActionScreen(SwipeDirection.START_TO_END, settings, onChange, onBack = { route = SwipeRoute.S47 })
            SwipeRoute.S48_END_TO_START -> ChooseSwipeActionScreen(SwipeDirection.END_TO_START, settings, onChange, onBack = { route = SwipeRoute.S47 })
        }
    }
}

/** "Swipe right" or "Swipe left": the physical direction of [direction] in this layout (stored start to end, so it mirrors). */
@Composable
fun directionLabel(direction: SwipeDirection): String {
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    return stringResource(if (direction.physicalSign(rtl) > 0) R.string.swipe_right else R.string.swipe_left)
}

@Composable
private fun sensitivityName(s: SwipeSensitivity) = stringResource(
    when (s) {
        SwipeSensitivity.LOW -> R.string.swipe_sensitivity_low
        SwipeSensitivity.NORMAL -> R.string.swipe_sensitivity_normal
        SwipeSensitivity.HIGH -> R.string.swipe_sensitivity_high
    },
)

@Composable
private fun presetName(p: SwipePreset) = stringResource(
    when (p) {
        SwipePreset.TRIAGE -> R.string.swipe_preset_triage
        SwipePreset.GMAIL_STYLE -> R.string.swipe_preset_gmail
        SwipePreset.TIDY -> R.string.swipe_preset_tidy
    },
)

/** A disc in the action's tonal colours with its icon: what the pill will look like, shown before the choice is made. */
@Composable
private fun ActionBadge(action: SwipeAction, modifier: Modifier = Modifier) {
    val colours = swipePillColours(action, available = true)
    Box(modifier.size(40.dp).clip(CircleShape).background(colours.container), contentAlignment = Alignment.Center) {
        Icon(swipeIcon(action, null), contentDescription = null, tint = colours.onContainer, modifier = Modifier.size(24.dp))
    }
}

/**
 * The live preview row of S47: a real row in the real swipe host that plays a looped demonstration of each direction (silent, and
 * stopped by a finger) and can be swiped by hand. Nothing it does touches mail: a committed action is shown and the row comes back.
 */
@Composable
private fun SwipePreview(settings: SwipeSettings, demo: Boolean) {
    val res = LocalContext.current.resources
    val noop = remember { SwipeCallbacks(onKeep = {}, onRemoved = { _, restore -> restore() }, onPark = {}, onNotAvailable = {}) }
    Surface(shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLow, modifier = Modifier.fillMaxWidth().testTag(SwipeTags.PREVIEW)) {
        Column(Modifier.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                stringResource(R.string.swipe_preview_label),
                Modifier.padding(horizontal = 16.dp),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            SwipeRowHost(
                settings = settings,
                enabled = true,
                alive = true,
                slotOf = { d -> SwipeSlot(settings.actionOf(d)) },
                iconOf = { a -> swipeIcon(a, null) },
                listScrolling = { false },
                callbacks = noop,
                parkResolution = null,
                onParkResolutionHandled = {},
                demo = if (demo) listOf(SwipeDirection.START_TO_END, SwipeDirection.END_TO_START) else null,
            ) { card ->
                MessageRow(
                    sender = res.getString(R.string.swipe_preview_sender),
                    subject = res.getString(R.string.swipe_preview_subject),
                    preview = res.getString(R.string.swipe_preview_snippet),
                    time = res.getString(R.string.swipe_preview_time),
                    state = MessageRowState(unread = true, encryption = EncryptionMark.NONE),
                    onClick = {},
                    card = card,
                )
            }
            Text(
                stringResource(R.string.swipe_preview_hint),
                Modifier.padding(horizontal = 16.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * S47 Swipe actions: the live preview, a picker for each direction (each opens S48), the sensitivity, the note that Delete forever
 * always asks, the presets and Reset. Every change applies at once.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SwipeActionsScreen(
    settings: SwipeSettings,
    onChange: (SwipeSettings) -> Unit,
    onBack: () -> Unit,
    onPick: (SwipeDirection) -> Unit,
    modifier: Modifier = Modifier,
    demo: Boolean = true,
) {
    val res = LocalContext.current.resources
    SettingsFrame(
        title = stringResource(R.string.swipe_title),
        onBack = onBack,
        modifier = modifier.testTag(SwipeTags.S47),
        preview = { SwipePreview(settings, demo) },
    ) {
        for (d in listOf(SwipeDirection.START_TO_END, SwipeDirection.END_TO_START)) {
            val action = settings.actionOf(d)
            val label = directionLabel(d)
            val name = swipeSettingName(res, action)
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(SwipeTags.picker(d)).clickable(role = Role.Button, onClick = { onPick(d) }).padding(vertical = 4.dp)
                    .semantics(mergeDescendants = true) { contentDescription = "$label. $name" },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                ActionBadge(action)
                Column(Modifier.weight(1f)) {
                    Text(label, style = MaterialTheme.typography.bodyLarge)
                    Text(name, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }

        SectionLabel(stringResource(R.string.swipe_sensitivity))
        ChoiceRow(
            options = SwipeSensitivity.entries,
            selected = settings.sensitivity,
            label = { sensitivityName(it) },
            onSelect = { onChange(settings.copy(sensitivity = it)) },
            modifier = Modifier.testTag("S47-sensitivity"),
        )

        Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            Box(Modifier.size(40.dp).clip(CircleShape).background(MaterialTheme.colorScheme.surfaceContainerHighest), contentAlignment = Alignment.Center) {
                Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(24.dp))
            }
            Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.swipe_forever_title), style = MaterialTheme.typography.bodyLarge)
                Text(stringResource(R.string.swipe_forever_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }

        SectionLabel(stringResource(R.string.swipe_presets))
        val matching = settings.matchingPreset()
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
            for (p in SwipePreset.entries) {
                FilterChip(
                    selected = matching == p,
                    onClick = { onChange(settings.copy(startToEnd = p.startToEnd, endToStart = p.endToStart)) },
                    label = { Text(presetName(p), maxLines = 1) },
                    leadingIcon = if (matching == p) ({ Icon(Icons.Filled.Check, contentDescription = null, Modifier.size(18.dp)) }) else null,
                    modifier = Modifier.heightIn(min = 48.dp).testTag(SwipeTags.preset(p)),
                )
            }
        }
        TextButton(onClick = { onChange(SwipeSettings.Default) }, modifier = Modifier.align(Alignment.End).heightIn(min = 48.dp).testTag(SwipeTags.RESET)) {
            Text(stringResource(R.string.swipe_reset))
        }
    }
}

/**
 * S48 Choose swipe action: a radio list for [direction]; each option shows the colour and icon the pill will use. Snooze and Move
 * to are not in the list until their sheets exist. Choosing the same action for both directions is allowed. Choosing applies at
 * once and returns to S47.
 */
@Composable
fun ChooseSwipeActionScreen(
    direction: SwipeDirection,
    settings: SwipeSettings,
    onChange: (SwipeSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val res = LocalContext.current.resources
    val current = settings.actionOf(direction)
    SettingsFrame(title = directionLabel(direction), onBack = onBack, modifier = modifier.testTag(SwipeTags.S48)) {
        Column(Modifier.fillMaxWidth().selectableGroup(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (a in SwipeAction.picker) {
                val name = swipeSettingName(res, a)
                val chosen = a == current
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(SwipeTags.option(a))
                        .selectable(selected = chosen, role = Role.RadioButton, onClick = { onChange(settings.with(direction, a)); onBack() })
                        .padding(vertical = 4.dp)
                        .semantics { if (chosen) stateDescription = res.getString(R.string.swipe_chosen_state) },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    ActionBadge(a)
                    Text(name, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                    RadioButton(selected = chosen, onClick = null)
                }
            }
        }
        Text(stringResource(R.string.swipe_choose_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
    }
}
