// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.design.CroftContrast
import com.example.croft.theme.LocalCroftMode
import com.example.croft.theme.McuSchemes
import com.example.croft.theme.ThemeJson
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStyle
import com.example.croft.theme.colorScheme
import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin

/** Test tags on S51. */
object CustomColourTags {
    const val SCREEN = "S51"
    const val HEX = "S51-hex"
    const val APPLY = "S51-apply"
    const val WHEEL = "S51-wheel"
    const val FROM_PHOTO = "S51-photo"
}

/** Picks a photo and hands back its seed colour (null: no usable colour). Implemented by the host (system picker). */
fun interface PhotoSeedPicker {
    fun pick(onSeed: (Long?) -> Unit)
}

private const val WHEEL_SEGMENTS = 72
private const val WHEEL_CHROMA = 48.0
private const val WHEEL_TONE = 62.0
private const val MIN_PICKED_CHROMA = 24.0
private const val HUE_STEP = 10.0

/** True when this seed and style give a scheme that looks nearly grey, so S51 can say so (docs/ux/theme.md). */
fun looksGrey(seed: Long, style: ThemeStyle): Boolean =
    style == ThemeStyle.NEUTRAL || style == ThemeStyle.MONOCHROME || McuSchemes.hctOf(seed).chroma < 12.0

/**
 * S51 Custom colour. A hue wheel, a hex field and From photo choose the seed; Style and Contrast shape the scheme; the
 * preview shows the draft and nothing changes in the app until Apply. The library keeps text on its containers readable
 * for any seed, so the note says low contrast is corrected automatically.
 */
@Composable
fun CustomColourScreen(
    settings: ThemeSettings,
    onApply: (ThemeSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    photoPicker: PhotoSeedPicker? = null,
) {
    var seed by remember { mutableStateOf(settings.effectiveSeed) }
    var style by remember { mutableStateOf(settings.style) }
    var contrast by remember { mutableStateOf(settings.contrast) }
    var hexText by remember { mutableStateOf(ThemeJson.hex(seed).removePrefix("#")) }
    var photoFailed by remember { mutableStateOf(false) }
    val hexValid = ThemeJson.parseHex(hexText) != null
    LaunchedEffect(seed) {
        if (ThemeJson.parseHex(hexText) != seed) hexText = ThemeJson.hex(seed).removePrefix("#")
    }
    val draft = remember(seed, style, contrast, settings) { settings.withCustomSeed(seed).copy(style = style, contrast = contrast) }
    val mode = LocalCroftMode.current
    val draftScheme = remember(seed, style, contrast, mode) { draft.colorScheme(mode) }

    SettingsFrame(
        title = stringResource(R.string.custom_title),
        onBack = onBack,
        modifier = modifier.testTag(CustomColourTags.SCREEN),
        preview = {
            // The draft only: the rest of the app keeps its theme until Apply.
            MaterialTheme(colorScheme = draftScheme, shapes = MaterialTheme.shapes, typography = MaterialTheme.typography) {
                AppearancePreview()
            }
        },
    ) {
        HueWheel(seed = seed, onSeed = { seed = it; photoFailed = false }, modifier = Modifier.align(Alignment.CenterHorizontally))

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(
                value = hexText,
                onValueChange = { text ->
                    val cleaned = text.removePrefix("#").take(6)
                    hexText = cleaned
                    ThemeJson.parseHex(cleaned)?.let { seed = it }
                },
                modifier = Modifier.weight(1f).testTag(CustomColourTags.HEX),
                label = { Text(stringResource(R.string.custom_hex)) },
                prefix = { Text("#") },
                singleLine = true,
                isError = !hexValid,
                supportingText = if (!hexValid) ({ Text(stringResource(R.string.custom_hex_error)) }) else null,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
            )
            if (photoPicker != null) {
                FilledTonalButton(
                    onClick = { photoPicker.pick { picked -> if (picked != null) { seed = picked; photoFailed = false } else photoFailed = true } },
                    modifier = Modifier.heightIn(min = 48.dp).testTag(CustomColourTags.FROM_PHOTO),
                ) { Text(stringResource(R.string.custom_from_photo)) }
            }
        }
        if (photoFailed) Text(stringResource(R.string.custom_photo_failed), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)

        SectionLabel(stringResource(R.string.custom_section_style))
        Wrap {
            ThemeStyle.entries.forEach { s ->
                FilterChip(selected = style == s, onClick = { style = s }, label = { Text(styleName(s)) })
            }
        }
        SectionLabel(stringResource(R.string.custom_section_contrast))
        ChoiceRow(options = CroftContrast.entries, selected = contrast, label = { contrastName(it) }, onSelect = { contrast = it })

        Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerHigh) {
            Text(
                stringResource(if (looksGrey(seed, style)) R.string.custom_note_grey else R.string.custom_note_adjusted),
                Modifier.padding(16.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Button(
            onClick = { onApply(draft); onBack() },
            enabled = hexValid,
            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(CustomColourTags.APPLY),
        ) { Text(stringResource(R.string.appearance_apply)) }
    }
}

/**
 * A ring of hues (computed with the colour library at one chroma and tone, so no colour is typed in) around the seed.
 * Drag or tap to choose a hue; keeps the seed's own chroma and tone, so a muted seed stays muted. Screen readers get the hue
 * in degrees and two actions to step it.
 */
@Composable
internal fun HueWheel(seed: Long, onSeed: (Long) -> Unit, modifier: Modifier = Modifier) {
    val hct = remember(seed) { McuSchemes.hctOf(seed) }
    val ring = remember { List(WHEEL_SEGMENTS) { Color(McuSchemes.fromHct(it * 360.0 / WHEEL_SEGMENTS, WHEEL_CHROMA, WHEEL_TONE)) } }
    val seedColour = Color(seed)
    val labelColour = Color(McuSchemes.fromHct(hct.hue, 8.0, if (hct.tone >= 60.0) 10.0 else 95.0))
    val outline = MaterialTheme.colorScheme.onSurface
    val description = stringResource(R.string.custom_wheel_description)
    val state = stringResource(R.string.custom_wheel_state, Math.round(hct.hue).toInt())
    val up = stringResource(R.string.custom_wheel_hue_up)
    val down = stringResource(R.string.custom_wheel_hue_down)

    fun pick(hue: Double) {
        val chroma = maxOf(hct.chroma, MIN_PICKED_CHROMA)
        onSeed(McuSchemes.fromHct(hue, chroma, hct.tone))
    }

    Box(
        modifier.size(248.dp).testTag(CustomColourTags.WHEEL)
            .semantics {
                contentDescription = description
                stateDescription = state
                customActions = listOf(
                    CustomAccessibilityAction(up) { pick(hct.hue + HUE_STEP); true },
                    CustomAccessibilityAction(down) { pick(hct.hue - HUE_STEP); true },
                )
            }
            .pointerInput(seed) {
                fun hueAt(p: Offset): Double {
                    val c = Offset(size.width / 2f, size.height / 2f)
                    val deg = Math.toDegrees(atan2((p.x - c.x).toDouble(), (c.y - p.y).toDouble()))
                    return (deg + 360.0) % 360.0
                }
                detectTapGestures { pick(hueAt(it)) }
            }
            .pointerInput(seed) {
                fun hueAt(p: Offset): Double {
                    val c = Offset(size.width / 2f, size.height / 2f)
                    return (Math.toDegrees(atan2((p.x - c.x).toDouble(), (c.y - p.y).toDouble())) + 360.0) % 360.0
                }
                detectDragGestures(onDragStart = { pick(hueAt(it)) }) { change, _ -> pick(hueAt(change.position)); change.consume() }
            },
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.size(248.dp)) {
            val stroke = 36.dp.toPx()
            val d = min(size.width, size.height) - stroke
            val topLeft = Offset(stroke / 2f, stroke / 2f)
            val sweep = 360f / WHEEL_SEGMENTS
            ring.forEachIndexed { i, c ->
                // Hue 0 is at the top, increasing clockwise; arc angle 0 is at 3 o'clock, so start 90 degrees earlier.
                drawArc(c, i * sweep - 90f - sweep / 2f, sweep + 0.5f, false, topLeft, Size(d, d), style = Stroke(stroke))
            }
            val r = d / 2f
            val a = (hct.hue - 90.0) * PI / 180.0
            val centre = Offset(size.width / 2f, size.height / 2f)
            val handle = Offset(centre.x + (r * cos(a)).toFloat(), centre.y + (r * sin(a)).toFloat())
            drawCircle(outline, 13.dp.toPx(), handle)
            drawCircle(seedColour, 9.dp.toPx(), handle)
            drawCircle(seedColour, r - stroke / 2f - 12.dp.toPx(), centre)
        }
        Text(stringResource(R.string.custom_seed), color = labelColour, style = MaterialTheme.typography.titleMedium)
    }
}
