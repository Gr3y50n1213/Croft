// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftSchemes

/** The three width classes the Appearance screens lay out for (docs/ux: compact, medium, expanded). */
enum class WidthClass { COMPACT, MEDIUM, EXPANDED;
    companion object {
        /** Compact below 600 dp, medium below 840 dp, expanded from 840 dp. */
        fun of(width: Dp): WidthClass = when {
            width < 600.dp -> COMPACT
            width < 840.dp -> MEDIUM
            else -> EXPANDED
        }
    }
}

/** Past this combined font scale, choices stack as a list instead of a segmented row, so labels never clip. */
internal const val STACK_FROM_FONT_SCALE = 1.3f

/** Below this window height the preview scrolls with the controls instead of staying pinned at the top. */
private val PinPreviewMinHeight = 640.dp
private val MediumContentWidth = 560.dp
private val ExpandedPreviewWidth = 360.dp
private val ScreenGutter = 16.dp

/**
 * The frame every settings screen shares: a top bar with Back (and optional actions), a snackbar host, and a body that is
 * one column on compact and medium widths and, on expanded widths, the [preview] pinned on the left with the controls
 * scrolling on the right. At a large font scale, or in a short window, the preview scrolls with the controls instead of
 * staying pinned.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SettingsFrame(
    title: String,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    snackbarHostState: SnackbarHostState? = null,
    actions: @Composable () -> Unit = {},
    preview: (@Composable () -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.appearance_back))
                    }
                },
                actions = { actions() },
            )
        },
        snackbarHost = { snackbarHostState?.let { SnackbarHost(it) } },
    ) { inner ->
        BoxWithConstraints(Modifier.fillMaxSize().padding(inner)) {
            val width = WidthClass.of(maxWidth)
            val bigText = LocalDensity.current.fontScale > STACK_FROM_FONT_SCALE
            // Pinned only while the controls keep room to work: not at large text and not on a short window.
            val pinPreview = !bigText && maxHeight >= PinPreviewMinHeight
            if (width == WidthClass.EXPANDED && preview != null) {
                Row(Modifier.fillMaxSize().padding(horizontal = ScreenGutter), horizontalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterHorizontally)) {
                    Box(Modifier.widthIn(max = ExpandedPreviewWidth).weight(1f, fill = false).verticalScroll(rememberScrollState()).padding(top = 8.dp)) { preview() }
                    Column(
                        Modifier.widthIn(max = MediumContentWidth).weight(1f).verticalScroll(rememberScrollState()).padding(vertical = 8.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                        content = content,
                    )
                }
            } else {
                val scroll = rememberScrollState()
                Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally) {
                    if (preview != null && pinPreview) {
                        Box(Modifier.widthIn(max = MediumContentWidth).fillMaxWidth().padding(horizontal = ScreenGutter, vertical = 8.dp)) { preview() }
                    }
                    Column(
                        Modifier.widthIn(max = MediumContentWidth).fillMaxWidth().weight(1f).verticalScroll(scroll).padding(horizontal = ScreenGutter, vertical = 8.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        if (preview != null && !pinPreview) preview()
                        content()
                    }
                }
            }
        }
    }
}

/** A section heading ("MODE", "COLOUR SOURCE"), announced as a heading. */
@Composable
internal fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier.padding(top = 8.dp).semantics { heading() },
    )
}

/**
 * One choice out of a few. A segmented row at normal text sizes (the wireframes); a stacked radio list when text is
 * large, so no label is cut off at 200 percent.
 */
@Composable
internal fun <T> ChoiceRow(
    options: List<T>,
    selected: T,
    label: @Composable (T) -> String,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (LocalDensity.current.fontScale > STACK_FROM_FONT_SCALE) {
        Column(modifier.selectableGroup()) {
            options.forEach { option ->
                RadioRow(title = label(option), hint = null, selected = option == selected, onClick = { onSelect(option) })
            }
        }
    } else {
        SingleChoiceSegmentedButtonRow(modifier.fillMaxWidth()) {
            options.forEachIndexed { index, option ->
                SegmentedButton(
                    selected = option == selected,
                    onClick = { onSelect(option) },
                    shape = SegmentedButtonDefaults.itemShape(index, options.size),
                    label = { Text(label(option), maxLines = 1, overflow = TextOverflow.Ellipsis) },
                )
            }
        }
    }
}

/** A radio row with a title and an optional one-line explanation, 48 dp tall at least. */
@Composable
internal fun RadioRow(title: String, hint: String?, selected: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().heightIn(min = 48.dp).selectable(selected = selected, onClick = onClick, role = Role.RadioButton).padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            if (hint != null) Text(hint, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        RadioButton(selected = selected, onClick = null)
    }
}

/** A tile with content on top and a label below; the selected tile has a thick primary outline and a container fill. */
@Composable
internal fun OptionTile(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    description: String = label,
    content: @Composable () -> Unit,
) {
    Surface(
        modifier = modifier.heightIn(min = 48.dp).semantics { contentDescription = description },
        selected = selected,
        onClick = onClick,
        shape = MaterialTheme.shapes.large,
        color = if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface,
        border = BorderStroke(if (selected) 2.dp else 1.dp, if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant),
    ) {
        Column(Modifier.padding(8.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            content()
            Text(label, style = MaterialTheme.typography.labelMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}

/**
 * A palette tile: a disc of the palette's primary, secondary and surface-container colours for the mode now showing, from
 * the generated tables (so the swatch is the exact colour the app will use), with a check when it is the one in use.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun PaletteSwatch(
    preset: CroftPreset,
    mode: CroftMode,
    selected: Boolean,
    name: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 48.dp,
) {
    val roles = CroftSchemes.roles(preset, mode)
    val ring = MaterialTheme.colorScheme.primary
    val description = stringResource(R.string.palette_swatch_description, name)
    val selectedText = if (selected) stringResource(R.string.appearance_selected) else ""
    Surface(
        modifier = modifier.size(maxOf(size, 48.dp)).semantics { contentDescription = description; stateDescription = selectedText },
        selected = selected,
        onClick = onClick,
        shape = androidx.compose.foundation.shape.CircleShape,
        color = Color.Transparent,
        border = if (selected) BorderStroke(3.dp, ring) else null,
    ) {
        Box(Modifier.padding(if (selected) 5.dp else 2.dp), contentAlignment = Alignment.Center) {
            Canvas(Modifier.fillMaxSize()) {
                val d = this.size.minDimension
                val box = Size(d, d)
                drawArc(Color(roles.primary), 90f, 180f, true, Offset.Zero, box, style = Fill)
                drawArc(Color(roles.secondaryContainer), 270f, 90f, true, Offset.Zero, box, style = Fill)
                drawArc(Color(roles.surfaceContainerHighest), 0f, 90f, true, Offset.Zero, box, style = Fill)
            }
            if (selected) Icon(Icons.Filled.Check, contentDescription = null, tint = Color(roles.onPrimary), modifier = Modifier.size(20.dp))
        }
    }
}

/** The overflow menu of S50: Reset, Export, Import. */
@Composable
internal fun OverflowMenu(items: List<Pair<String, () -> Unit>>) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(Icons.Filled.MoreVert, contentDescription = stringResource(R.string.appearance_more_options))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            items.forEach { (label, action) ->
                DropdownMenuItem(text = { Text(label) }, onClick = { open = false; action() })
            }
        }
    }
}

/** Wrapping row for chips and tiles, with 8 dp gaps. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun Wrap(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    FlowRow(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { content() }
}
