// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.theme.AvatarStyle
import com.example.croft.theme.DensityLevel
import com.example.croft.theme.ListStyle
import com.example.croft.theme.ReadingFont
import com.example.croft.theme.Roundness
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.UiFont
import com.example.croft.theme.UnreadEmphasis
import com.example.croft.theme.family

/** Test tags on S52. */
object LayoutTags {
    const val SCREEN = "S52"
    const val TEXT_SIZE = "S52-text-size"
    const val DENSITY_PREFIX = "S52-density-"
}

@Composable
private fun densityName(d: DensityLevel) = stringResource(
    when (d) {
        DensityLevel.COMFORTABLE -> R.string.layout_density_comfortable
        DensityLevel.DEFAULT -> R.string.layout_density_default
        DensityLevel.COMPACT -> R.string.layout_density_compact
    },
)

/** A diagram of rows at a density: bars the way the wireframe draws them, more of them when rows are shorter. */
@Composable
private fun DensityDiagram(d: DensityLevel) {
    val bars = when (d) { DensityLevel.COMFORTABLE -> 3; DensityLevel.DEFAULT -> 4; DensityLevel.COMPACT -> 6 }
    Column(Modifier.width(72.dp).padding(4.dp), verticalArrangement = Arrangement.spacedBy(if (d == DensityLevel.COMPACT) 3.dp else 5.dp)) {
        repeat(bars) {
            Box(Modifier.fillMaxWidth().height(if (d == DensityLevel.COMPACT) 5.dp else 8.dp).background(MaterialTheme.colorScheme.outlineVariant, MaterialTheme.shapes.extraSmall))
        }
    }
}

@Composable
private fun ListStyleDiagram(style: ListStyle) {
    Box(Modifier.width(96.dp).height(52.dp).padding(4.dp), contentAlignment = Alignment.Center) {
        if (style == ListStyle.CONTAINER) {
            Box(Modifier.fillMaxWidth().height(40.dp).background(MaterialTheme.colorScheme.surfaceContainerHighest, MaterialTheme.shapes.medium))
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                repeat(3) { HorizontalDivider(Modifier.width(88.dp), color = MaterialTheme.colorScheme.outline) }
            }
        }
    }
}

/** S52 Layout and density: density, roundness, list style, avatars, unread emphasis, text size and fonts. */
@Composable
fun LayoutDensityScreen(
    settings: ThemeSettings,
    onChange: (ThemeSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    SettingsFrame(
        title = stringResource(R.string.layout_title),
        onBack = onBack,
        modifier = modifier.testTag(LayoutTags.SCREEN),
        preview = { AppearancePreview(showSnackbar = false) },
    ) {
        SectionLabel(stringResource(R.string.layout_section_density))
        Wrap {
            DensityLevel.entries.forEach { d ->
                val name = densityName(d)
                OptionTile(
                    label = name, selected = settings.density == d, onClick = { onChange(settings.copy(density = d)) },
                    modifier = Modifier.testTag(LayoutTags.DENSITY_PREFIX + d.id),
                    description = stringResource(R.string.layout_density_description, name, d.rowDp),
                ) { DensityDiagram(d) }
            }
        }

        SectionLabel(stringResource(R.string.layout_section_roundness))
        ChoiceRow(
            options = Roundness.entries, selected = settings.roundness,
            label = { stringResource(when (it) { Roundness.TIGHT -> R.string.layout_roundness_tight; Roundness.STANDARD -> R.string.layout_roundness_standard; Roundness.ROUND -> R.string.layout_roundness_round }) },
            onSelect = { onChange(settings.copy(roundness = it)) },
        )

        SectionLabel(stringResource(R.string.layout_section_list_style))
        Wrap {
            ListStyle.entries.forEach { l ->
                OptionTile(
                    label = stringResource(if (l == ListStyle.CONTAINER) R.string.layout_list_container else R.string.layout_list_flat),
                    selected = settings.listStyle == l, onClick = { onChange(settings.copy(listStyle = l)) },
                ) { ListStyleDiagram(l) }
            }
        }

        SectionLabel(stringResource(R.string.layout_section_avatars))
        ChoiceRow(
            options = AvatarStyle.entries, selected = settings.avatars,
            label = { stringResource(when (it) { AvatarStyle.LETTERS -> R.string.layout_avatars_letters; AvatarStyle.PHOTOS -> R.string.layout_avatars_photos; AvatarStyle.HIDDEN -> R.string.layout_avatars_hidden }) },
            onSelect = { onChange(settings.copy(avatars = it)) },
        )
        if (settings.avatars == AvatarStyle.PHOTOS) Hint(stringResource(R.string.layout_avatars_photos_hint))

        SectionLabel(stringResource(R.string.layout_section_unread))
        ChoiceRow(
            options = UnreadEmphasis.entries, selected = settings.unreadEmphasis,
            label = { stringResource(when (it) { UnreadEmphasis.BOLD_DOT -> R.string.layout_unread_bold_dot; UnreadEmphasis.BOLD -> R.string.layout_unread_bold; UnreadEmphasis.DOT -> R.string.layout_unread_dot }) },
            onSelect = { onChange(settings.copy(unreadEmphasis = it)) },
        )
        Hint(stringResource(R.string.layout_unread_hint))

        SectionLabel(stringResource(R.string.layout_section_text_size))
        val percent = Math.round(settings.textScale * 100f)
        Slider(
            value = settings.textScale,
            onValueChange = { onChange(settings.copy(textScale = ThemeSettings.clampTextScale(it))) },
            valueRange = ThemeSettings.TEXT_SCALE_MIN..ThemeSettings.TEXT_SCALE_MAX,
            steps = Math.round((ThemeSettings.TEXT_SCALE_MAX - ThemeSettings.TEXT_SCALE_MIN) / ThemeSettings.TEXT_SCALE_STEP) - 1,
            modifier = Modifier.fillMaxWidth().testTag(LayoutTags.TEXT_SIZE),
        )
        Text(stringResource(R.string.layout_text_size_value, percent), Modifier.align(Alignment.CenterHorizontally), style = MaterialTheme.typography.titleMedium)
        Hint(stringResource(R.string.layout_text_size_hint))

        SectionLabel(stringResource(R.string.layout_section_ui_font))
        Column(Modifier.selectableGroup()) {
            UiFont.entries.forEach { f ->
                RadioRow(
                    title = stringResource(when (f) { UiFont.SYSTEM -> R.string.layout_font_system; UiFont.CROFT_SANS -> R.string.layout_font_croft_sans; UiFont.ATKINSON -> R.string.layout_font_atkinson }),
                    hint = null, selected = settings.uiFont == f, onClick = { onChange(settings.copy(uiFont = f)) },
                )
            }
        }

        SectionLabel(stringResource(R.string.layout_section_reading_font))
        Column(Modifier.selectableGroup()) {
            ReadingFont.entries.forEach { f ->
                RadioRow(
                    title = stringResource(when (f) { ReadingFont.SYSTEM -> R.string.layout_font_system; ReadingFont.SERIF -> R.string.layout_font_serif; ReadingFont.ATKINSON -> R.string.layout_font_atkinson }),
                    hint = null, selected = settings.readingFont == f, onClick = { onChange(settings.copy(readingFont = f)) },
                )
            }
        }
        Text(stringResource(R.string.layout_font_sample), style = MaterialTheme.typography.bodyLarge.copy(fontFamily = settings.readingFont.family()))
        Hint(stringResource(R.string.layout_reading_hint))
    }
}

@Composable
private fun Hint(text: String) {
    Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
}
