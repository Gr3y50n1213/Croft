// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import android.content.Context
import android.os.Build
import android.provider.Settings
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Density
import com.example.croft.R
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftMotion
import com.example.croft.design.croftMotionScheme
import com.example.croft.design.toColorScheme

/** The look and feel settings, for every composable under the theme. The one way settings reach the UI. */
val LocalThemeSettings = staticCompositionLocalOf { ThemeSettings.Default }

/** What the Animation level setting resolves to for this device right now. */
enum class MotionLevel {
    /** Expressive springs. */
    FULL,
    /** The standard springs; travel becomes short fades; theme changes swap at once (docs/ux/motion.md). */
    REDUCED,
    /** Every change is instant. */
    OFF,
}

/** Follow system reads the animator duration scale: 0 ("remove animations") is Off, anything else is Full. */
fun resolveMotionLevel(pref: AnimationLevel, systemAnimatorScale: Float): MotionLevel = when (pref) {
    AnimationLevel.FULL -> MotionLevel.FULL
    AnimationLevel.REDUCED -> MotionLevel.REDUCED
    AnimationLevel.OFF -> MotionLevel.OFF
    AnimationLevel.SYSTEM -> if (systemAnimatorScale == 0f) MotionLevel.OFF else MotionLevel.FULL
}

/** The resolved motion level; custom animations check it (`!= FULL` means use the reduced-motion variant). */
val LocalMotionLevel = staticCompositionLocalOf { MotionLevel.FULL }

/** Light, dark or pure black, as resolved from the settings and the system: what the colours on screen now are. */
val LocalCroftMode = staticCompositionLocalOf { CroftMode.LIGHT }

/** The font for message bodies (S52 "Reading font"); it never changes the interface. */
val LocalReadingFontFamily = staticCompositionLocalOf<FontFamily> { FontFamily.Default }

private fun systemAnimatorScale(context: Context): Float =
    try {
        Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
    } catch (e: RuntimeException) {
        1f
    }

private val CroftSansFamily = FontFamily(
    Font(R.font.croft_sans_regular, FontWeight.Normal),
    Font(R.font.croft_sans_medium, FontWeight.Medium),
    Font(R.font.croft_sans_bold, FontWeight.Bold),
)

private val AtkinsonFamily = FontFamily(
    Font(R.font.atkinson_hyperlegible_regular, FontWeight.Normal),
    Font(R.font.atkinson_hyperlegible_bold, FontWeight.Bold),
)

/** The interface font family, or null for the system font. */
fun UiFont.family(): FontFamily? = when (this) {
    UiFont.SYSTEM -> null
    UiFont.CROFT_SANS -> CroftSansFamily
    UiFont.ATKINSON -> AtkinsonFamily
}

/** The reading font family. */
fun ReadingFont.family(): FontFamily = when (this) {
    ReadingFont.SYSTEM -> FontFamily.Default
    ReadingFont.SERIF -> FontFamily.Serif
    ReadingFont.ATKINSON -> AtkinsonFamily
}

/** Material typography with the interface font on every style. */
internal fun croftTypography(font: UiFont): Typography {
    val base = Typography()
    val family = font.family() ?: return base
    fun TextStyle.f() = copy(fontFamily = family)
    return base.copy(
        displayLarge = base.displayLarge.f(), displayMedium = base.displayMedium.f(), displaySmall = base.displaySmall.f(),
        headlineLarge = base.headlineLarge.f(), headlineMedium = base.headlineMedium.f(), headlineSmall = base.headlineSmall.f(),
        titleLarge = base.titleLarge.f(), titleMedium = base.titleMedium.f(), titleSmall = base.titleSmall.f(),
        bodyLarge = base.bodyLarge.f(), bodyMedium = base.bodyMedium.f(), bodySmall = base.bodySmall.f(),
        labelLarge = base.labelLarge.f(), labelMedium = base.labelMedium.f(), labelSmall = base.labelSmall.f(),
    )
}

/** Opaque colour roles to a Compose scheme for light, dark or pure black. */
internal fun ThemeSettings.colorScheme(mode: CroftMode): ColorScheme = ThemeEngine.roles(this, mode).toColorScheme(mode)

/**
 * The Compose colour scheme for [s]: the wallpaper's dynamic colours on Android 12 and later when chosen, otherwise
 * the preset table or the runtime scheme. Pure black darkens the base surfaces of a dynamic scheme the same way.
 */
internal fun resolveColorScheme(s: ThemeSettings, systemDark: Boolean, context: Context): ColorScheme {
    val mode = ThemeEngine.resolveMode(s, systemDark)
    if (s.source == ColourSource.WALLPAPER && wallpaperColoursAvailable()) {
        val dynamic = wallpaperScheme(context, mode)
        return if (mode == CroftMode.PURE_BLACK) {
            dynamic.copy(
                background = Color.Black, surface = Color.Black, surfaceDim = Color.Black, surfaceContainerLowest = Color.Black,
            )
        } else dynamic
    }
    return s.colorScheme(mode)
}

@androidx.annotation.RequiresApi(Build.VERSION_CODES.S)
private fun wallpaperScheme(context: Context, mode: CroftMode): ColorScheme =
    if (mode == CroftMode.LIGHT) dynamicLightColorScheme(context) else dynamicDarkColorScheme(context)

/** Whether the wallpaper colour source exists on this device (Android 12 and later; hidden below that). */
fun wallpaperColoursAvailable(sdkInt: Int = Build.VERSION.SDK_INT): Boolean = sdkInt >= Build.VERSION_CODES.S

/** Every role, from [a] to [b] by [f]. The colours are all that change; no size or position does. */
internal fun lerpColorScheme(a: ColorScheme, b: ColorScheme, f: Float): ColorScheme = a.copy(
    primary = lerp(a.primary, b.primary, f), onPrimary = lerp(a.onPrimary, b.onPrimary, f),
    primaryContainer = lerp(a.primaryContainer, b.primaryContainer, f),
    onPrimaryContainer = lerp(a.onPrimaryContainer, b.onPrimaryContainer, f),
    inversePrimary = lerp(a.inversePrimary, b.inversePrimary, f),
    secondary = lerp(a.secondary, b.secondary, f), onSecondary = lerp(a.onSecondary, b.onSecondary, f),
    secondaryContainer = lerp(a.secondaryContainer, b.secondaryContainer, f),
    onSecondaryContainer = lerp(a.onSecondaryContainer, b.onSecondaryContainer, f),
    tertiary = lerp(a.tertiary, b.tertiary, f), onTertiary = lerp(a.onTertiary, b.onTertiary, f),
    tertiaryContainer = lerp(a.tertiaryContainer, b.tertiaryContainer, f),
    onTertiaryContainer = lerp(a.onTertiaryContainer, b.onTertiaryContainer, f),
    background = lerp(a.background, b.background, f), onBackground = lerp(a.onBackground, b.onBackground, f),
    surface = lerp(a.surface, b.surface, f), onSurface = lerp(a.onSurface, b.onSurface, f),
    surfaceVariant = lerp(a.surfaceVariant, b.surfaceVariant, f),
    onSurfaceVariant = lerp(a.onSurfaceVariant, b.onSurfaceVariant, f),
    surfaceTint = lerp(a.surfaceTint, b.surfaceTint, f),
    inverseSurface = lerp(a.inverseSurface, b.inverseSurface, f),
    inverseOnSurface = lerp(a.inverseOnSurface, b.inverseOnSurface, f),
    error = lerp(a.error, b.error, f), onError = lerp(a.onError, b.onError, f),
    errorContainer = lerp(a.errorContainer, b.errorContainer, f),
    onErrorContainer = lerp(a.onErrorContainer, b.onErrorContainer, f),
    outline = lerp(a.outline, b.outline, f), outlineVariant = lerp(a.outlineVariant, b.outlineVariant, f),
    scrim = lerp(a.scrim, b.scrim, f),
    surfaceBright = lerp(a.surfaceBright, b.surfaceBright, f),
    surfaceContainer = lerp(a.surfaceContainer, b.surfaceContainer, f),
    surfaceContainerHigh = lerp(a.surfaceContainerHigh, b.surfaceContainerHigh, f),
    surfaceContainerHighest = lerp(a.surfaceContainerHighest, b.surfaceContainerHighest, f),
    surfaceContainerLow = lerp(a.surfaceContainerLow, b.surfaceContainerLow, f),
    surfaceContainerLowest = lerp(a.surfaceContainerLowest, b.surfaceContainerLowest, f),
    surfaceDim = lerp(a.surfaceDim, b.surfaceDim, f),
    primaryFixed = lerp(a.primaryFixed, b.primaryFixed, f), primaryFixedDim = lerp(a.primaryFixedDim, b.primaryFixedDim, f),
    onPrimaryFixed = lerp(a.onPrimaryFixed, b.onPrimaryFixed, f),
    onPrimaryFixedVariant = lerp(a.onPrimaryFixedVariant, b.onPrimaryFixedVariant, f),
    secondaryFixed = lerp(a.secondaryFixed, b.secondaryFixed, f),
    secondaryFixedDim = lerp(a.secondaryFixedDim, b.secondaryFixedDim, f),
    onSecondaryFixed = lerp(a.onSecondaryFixed, b.onSecondaryFixed, f),
    onSecondaryFixedVariant = lerp(a.onSecondaryFixedVariant, b.onSecondaryFixedVariant, f),
    tertiaryFixed = lerp(a.tertiaryFixed, b.tertiaryFixed, f),
    tertiaryFixedDim = lerp(a.tertiaryFixedDim, b.tertiaryFixedDim, f),
    onTertiaryFixed = lerp(a.onTertiaryFixed, b.onTertiaryFixed, f),
    onTertiaryFixedVariant = lerp(a.onTertiaryFixedVariant, b.onTertiaryFixedVariant, f),
)

private class Shown(var scheme: ColorScheme)

/**
 * The scheme to draw: [target] at once when motion is Reduced or Off (an instant swap, docs/ux/motion.md), otherwise a
 * cross-fade from what was on screen to [target] with the Effects slow spring. A change in the middle of a fade starts
 * from the colours then showing, so nothing jumps back. Only colours change, so no layout moves.
 */
@Composable
internal fun rememberCrossfadedColorScheme(target: ColorScheme, level: MotionLevel): ColorScheme {
    val shown = remember { Shown(target) }
    if (level != MotionLevel.FULL) {
        SideEffect { shown.scheme = target }
        return target
    }
    val from = remember(target) { shown.scheme }
    val progress = remember(target) { Animatable(if (from == target) 1f else 0f) }
    LaunchedEffect(target) {
        if (progress.value < 1f) progress.animateTo(1f, CroftMotion.effectsSlow.spec())
    }
    val current = if (progress.value >= 1f) target else lerpColorScheme(from, target, progress.value.coerceIn(0f, 1f))
    SideEffect { shown.scheme = current }
    return current
}

/**
 * The app theme. Colours from the settings (generated tables, runtime generation or the wallpaper), corners scaled by
 * Roundness, the interface font, and the text size multiplied with the system font size. Settings and the resolved
 * motion level reach the content through [LocalThemeSettings] and [LocalMotionLevel].
 */
@Composable
fun CroftThemeProvider(
    settings: ThemeSettings = ThemeSettings.Default,
    systemDark: Boolean = isSystemInDarkTheme(),
    motionLevel: MotionLevel = resolveMotionLevel(settings.animation, systemAnimatorScale(LocalContext.current)),
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val target = remember(
        settings.mode, settings.pureBlack, settings.source, settings.effectiveSeed, settings.preset,
        settings.style, settings.contrast, systemDark, context,
    ) { resolveColorScheme(settings, systemDark, context) }
    val scheme = rememberCrossfadedColorScheme(target, motionLevel)
    val shapes = remember(settings.roundness) { scaledShapes(settings.roundness) }
    val typography = remember(settings.uiFont) { croftTypography(settings.uiFont) }
    val outer = LocalDensity.current
    val density = remember(outer, settings.textScale) {
        Density(outer.density, outer.fontScale * ThemeSettings.clampTextScale(settings.textScale))
    }
    CompositionLocalProvider(
        LocalThemeSettings provides settings,
        LocalMotionLevel provides motionLevel,
        LocalCroftMode provides ThemeEngine.resolveMode(settings, systemDark),
        LocalReadingFontFamily provides settings.readingFont.family(),
        LocalDensity provides density,
    ) {
        MaterialTheme(
            colorScheme = scheme,
            shapes = shapes,
            typography = typography,
            motionScheme = croftMotionScheme(reducedMotion = motionLevel != MotionLevel.FULL),
            content = content,
        )
    }
}

internal fun scaledShapes(r: Roundness): Shapes = Shapes().scaledBy(r.scale)
