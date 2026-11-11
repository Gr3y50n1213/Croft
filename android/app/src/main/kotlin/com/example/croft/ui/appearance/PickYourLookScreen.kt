// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.design.CroftPreset
import com.example.croft.theme.ColourSource
import com.example.croft.theme.LocalCroftMode
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.wallpaperColoursAvailable

/** Test tags on S08. */
object PickLookTags {
    const val SCREEN = "S08"
    const val CONTINUE = "S08-continue"
    const val WALLPAPER = "S08-wallpaper"
    const val PALETTE_PREFIX = "S08-palette-"
}

/**
 * S08 Pick your look (onboarding). A live preview, System, Light or Dark, eight palettes and, on Android 12 and later,
 * the wallpaper's colours offered first. Every tap applies at once (the theme cross-fades); Continue moves on.
 */
@Composable
fun PickYourLookScreen(
    settings: ThemeSettings,
    onChange: (ThemeSettings) -> Unit,
    onBack: () -> Unit,
    onContinue: () -> Unit,
    modifier: Modifier = Modifier,
    wallpaperAvailable: Boolean = wallpaperColoursAvailable(),
) {
    val mode = LocalCroftMode.current
    SettingsFrame(
        title = stringResource(R.string.look_title),
        onBack = onBack,
        modifier = modifier.testTag(PickLookTags.SCREEN),
        preview = { AppearancePreview() },
    ) {
        ChoiceRow(
            options = listOf(ModePreference.SYSTEM, ModePreference.LIGHT, ModePreference.DARK), selected = settings.mode,
            label = { stringResource(when (it) { ModePreference.SYSTEM -> R.string.appearance_mode_system; ModePreference.LIGHT -> R.string.appearance_mode_light; ModePreference.DARK -> R.string.appearance_mode_dark }) },
            onSelect = { onChange(settings.copy(mode = it)) },
        )
        SectionLabel(stringResource(R.string.look_section_colour))
        if (wallpaperAvailable) {
            FilledTonalButton(
                onClick = { onChange(settings.copy(source = ColourSource.WALLPAPER)) },
                modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(PickLookTags.WALLPAPER),
            ) { Text(stringResource(R.string.look_wallpaper)) }
        }
        Wrap {
            CroftPreset.entries.forEach { preset ->
                val name = presetName(preset)
                PaletteSwatch(
                    preset = preset, mode = mode, name = name, size = 56.dp,
                    selected = settings.source == ColourSource.PRESET && settings.preset == preset,
                    onClick = { onChange(settings.withPreset(preset)) },
                    modifier = Modifier.testTag(PickLookTags.PALETTE_PREFIX + preset.id),
                )
            }
        }
        val shown = if (settings.source == ColourSource.PRESET) presetName(settings.preset) else stringResource(
            if (settings.source == ColourSource.WALLPAPER) R.string.appearance_source_wallpaper else R.string.palette_custom,
        )
        Text(shown, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(start = 4.dp))
        Button(onClick = onContinue, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(PickLookTags.CONTINUE)) {
            Text(stringResource(R.string.look_continue))
        }
    }
}
