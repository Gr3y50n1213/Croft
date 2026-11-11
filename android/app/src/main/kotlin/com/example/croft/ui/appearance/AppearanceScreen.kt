// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.selection.toggleable
import com.example.croft.R
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftTokens
import com.example.croft.theme.ColourSource
import com.example.croft.theme.LocalCroftMode
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStyle
import com.example.croft.theme.wallpaperColoursAvailable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Test tags on S50 for the controls the tests drive. */
object AppearanceTags {
    const val SCREEN = "S50"
    const val PURE_BLACK = "S50-pure-black"
    const val PALETTE_PREFIX = "S50-palette-"
    const val RESET_CONFIRM = "S50-reset-confirm"
}

/** The plain name of a palette, for labels. */
@Composable
internal fun presetName(p: CroftPreset): String = stringResource(
    when (p) {
        CroftPreset.MOSS -> R.string.palette_moss
        CroftPreset.WAX -> R.string.palette_wax
        CroftPreset.SLATE -> R.string.palette_slate
        CroftPreset.HEATHER -> R.string.palette_heather
        CroftPreset.OCHRE -> R.string.palette_ochre
        CroftPreset.PEAT -> R.string.palette_peat
        CroftPreset.TIDE -> R.string.palette_tide
        CroftPreset.ROWAN -> R.string.palette_rowan
    },
)

@Composable
internal fun styleName(s: ThemeStyle): String = stringResource(
    when (s) {
        ThemeStyle.TONAL_SPOT -> R.string.style_tonal_spot
        ThemeStyle.NEUTRAL -> R.string.style_neutral
        ThemeStyle.VIBRANT -> R.string.style_vibrant
        ThemeStyle.EXPRESSIVE -> R.string.style_expressive
        ThemeStyle.FIDELITY -> R.string.style_fidelity
        ThemeStyle.MONOCHROME -> R.string.style_monochrome
    },
)

@Composable
internal fun contrastName(c: CroftContrast): String = stringResource(
    when (c) {
        CroftContrast.STANDARD -> R.string.appearance_contrast_standard
        CroftContrast.MEDIUM -> R.string.appearance_contrast_medium
        CroftContrast.HIGH -> R.string.appearance_contrast_high
    },
)

/** How long the Undo snackbar after a palette change stays: the same Undo window the mail actions use (5 s by default). */
internal val PaletteUndoMillis: Long = CroftTokens.Swipe.UndoWindowSeconds.default * 1000L

/** Shows [message] with Undo for [PaletteUndoMillis]; [onUndo] runs if the person taps Undo in time. */
internal fun CoroutineScope.showUndo(host: SnackbarHostState, message: String, undo: String, onUndo: () -> Unit): Job = launch {
    val shown = launch {
        val result = host.showSnackbar(message, actionLabel = undo, withDismissAction = false, duration = SnackbarDuration.Indefinite)
        if (result == SnackbarResult.ActionPerformed) onUndo()
    }
    delay(PaletteUndoMillis)
    if (shown.isActive) {
        host.currentSnackbarData?.dismiss()
    }
}

/**
 * S50 Appearance. Live preview at the top, then mode, pure black, colour source, palettes, style and contrast, with links
 * to S51, S52 and S53. Every change calls [onChange] and shows in the preview and the app at once (the theme cross-fades).
 * Applying a palette offers Undo for 5 s; Reset asks first. [onImportTheme] and [onExportTheme] are null when the
 * platform cannot pick files (then the menu hides them). Stateless: the caller owns [settings].
 */
@Composable
fun AppearanceScreen(
    settings: ThemeSettings,
    onChange: (ThemeSettings) -> Unit,
    onBack: () -> Unit,
    onOpenCustomColour: () -> Unit,
    onOpenLayout: () -> Unit,
    onOpenMotion: () -> Unit,
    onReset: () -> Unit,
    modifier: Modifier = Modifier,
    onExportTheme: (() -> Unit)? = null,
    onImportTheme: (() -> Unit)? = null,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
    wallpaperAvailable: Boolean = wallpaperColoursAvailable(),
) {
    val scope = rememberCoroutineScope()
    var confirmReset by remember { mutableStateOf(false) }
    val undo = stringResource(R.string.appearance_undo)
    val appliedFormat = stringResource(R.string.appearance_palette_applied, "%1\$s")
    val customName = stringResource(R.string.palette_custom)
    val mode = LocalCroftMode.current

    val menu = buildList {
        add(stringResource(R.string.appearance_reset) to { confirmReset = true })
        if (onExportTheme != null) add(stringResource(R.string.appearance_export) to onExportTheme)
        if (onImportTheme != null) add(stringResource(R.string.appearance_import) to onImportTheme)
    }

    SettingsFrame(
        title = stringResource(R.string.appearance_title),
        onBack = onBack,
        modifier = modifier.testTag(AppearanceTags.SCREEN),
        snackbarHostState = snackbarHostState,
        actions = { OverflowMenu(menu) },
        preview = { AppearancePreview() },
    ) {
        SectionLabel(stringResource(R.string.appearance_section_mode))
        ChoiceRow(
            options = ModePreference.entries, selected = settings.mode,
            label = { stringResource(when (it) { ModePreference.SYSTEM -> R.string.appearance_mode_system; ModePreference.LIGHT -> R.string.appearance_mode_light; ModePreference.DARK -> R.string.appearance_mode_dark }) },
            onSelect = { onChange(settings.copy(mode = it)) },
        )
        // Pure black only applies in dark, so the switch is shown while dark can be in force and off otherwise.
        val darkPossible = settings.mode != ModePreference.LIGHT
        Row(
            Modifier.fillMaxWidth().heightIn(min = 56.dp).toggleable(
                value = settings.pureBlack, enabled = darkPossible, role = Role.Switch,
                onValueChange = { onChange(settings.copy(pureBlack = it)) },
            ).testTag(AppearanceTags.PURE_BLACK),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            androidx.compose.foundation.layout.Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.appearance_pure_black), style = MaterialTheme.typography.bodyLarge)
                Text(stringResource(R.string.appearance_pure_black_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Switch(checked = settings.pureBlack, onCheckedChange = null, enabled = darkPossible)
        }

        SectionLabel(stringResource(R.string.appearance_section_source))
        val sources = buildList {
            add(ColourSource.PRESET)
            if (wallpaperAvailable) add(ColourSource.WALLPAPER)
            add(ColourSource.CUSTOM)
        }
        ChoiceRow(
            options = sources, selected = settings.source,
            label = { stringResource(when (it) { ColourSource.PRESET -> R.string.appearance_source_croft; ColourSource.WALLPAPER -> R.string.appearance_source_wallpaper; ColourSource.CUSTOM -> R.string.appearance_source_custom }) },
            onSelect = { src ->
                if (src == ColourSource.CUSTOM) onOpenCustomColour() else onChange(settings.copy(source = src))
            },
        )
        if (settings.source == ColourSource.WALLPAPER) {
            Text(stringResource(R.string.appearance_wallpaper_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }

        if (settings.source != ColourSource.WALLPAPER) {
            SectionLabel(stringResource(R.string.appearance_section_palette))
            Wrap {
                CroftPreset.entries.forEach { preset ->
                    val name = presetName(preset)
                    val selected = settings.source == ColourSource.PRESET && settings.preset == preset
                    PaletteSwatch(
                        preset = preset, mode = mode, selected = selected, name = name,
                        modifier = Modifier.testTag(AppearanceTags.PALETTE_PREFIX + preset.id),
                        onClick = {
                            if (!selected) {
                                val before = settings
                                onChange(settings.withPreset(preset))
                                scope.showUndo(snackbarHostState, appliedFormat.replace("%1\$s", name), undo) { onChange(before) }
                            }
                        },
                    )
                }
            }
            if (settings.source == ColourSource.CUSTOM) {
                Text(customName, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }

            SectionLabel(stringResource(R.string.appearance_section_style))
            Wrap {
                ThemeStyle.entries.forEach { style ->
                    androidx.compose.material3.FilterChip(
                        selected = settings.style == style, onClick = { onChange(settings.copy(style = style)) },
                        label = { Text(styleName(style)) },
                    )
                }
            }
        }

        SectionLabel(stringResource(R.string.appearance_section_contrast))
        ChoiceRow(
            options = CroftContrast.entries, selected = settings.contrast,
            label = { contrastName(it) },
            onSelect = { onChange(settings.copy(contrast = it)) },
        )

        LinkRow(stringResource(R.string.appearance_link_custom), onOpenCustomColour)
        LinkRow(stringResource(R.string.appearance_link_layout), onOpenLayout)
        LinkRow(stringResource(R.string.appearance_link_motion), onOpenMotion)
    }

    if (confirmReset) {
        AlertDialog(
            onDismissRequest = { confirmReset = false },
            title = { Text(stringResource(R.string.appearance_reset_title)) },
            text = { Text(stringResource(R.string.appearance_reset_body)) },
            confirmButton = {
                TextButton(
                    onClick = { confirmReset = false; onReset() },
                    modifier = Modifier.testTag(AppearanceTags.RESET_CONFIRM),
                ) { Text(stringResource(R.string.appearance_reset_confirm)) }
            },
            dismissButton = { TextButton(onClick = { confirmReset = false }) { Text(stringResource(R.string.appearance_cancel)) } },
        )
    }
}

/** A row that opens another screen: label, chevron, 56 dp tall at least. */
@Composable
internal fun LinkRow(label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button, onClick = onClick).padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
        Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null)
    }
}
