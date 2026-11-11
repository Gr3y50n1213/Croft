// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftTokens

/** An option with a stable id: the id is what the stored JSON holds (docs/ux/theme.md, "Stored shape"). */
interface StoredOption {
    val id: String
}

/** Light or dark follows the system, or is fixed. Pure black is a separate switch that only applies in dark. */
enum class ModePreference(override val id: String) : StoredOption { SYSTEM("system"), LIGHT("light"), DARK("dark") }

/** Where the colours come from: a Croft palette, the wallpaper (Android 12 and later), or a custom seed. */
enum class ColourSource(override val id: String) : StoredOption { PRESET("preset"), WALLPAPER("wallpaper"), CUSTOM("custom") }

/** How colourful a generated scheme is; one Material Color Utilities scheme class each. */
enum class ThemeStyle(override val id: String) : StoredOption {
    TONAL_SPOT("tonalSpot"),
    NEUTRAL("neutral"),
    VIBRANT("vibrant"),
    EXPRESSIVE("expressive"),
    FIDELITY("fidelity"),
    MONOCHROME("monochrome"),
}

enum class DensityLevel(override val id: String) : StoredOption {
    COMFORTABLE("comfortable"),
    DEFAULT("default"),
    COMPACT("compact");

    val rowDp: Int get() = when (this) {
        COMFORTABLE -> CroftTokens.Density.Comfortable.rowDp
        DEFAULT -> CroftTokens.Density.Default.rowDp
        COMPACT -> CroftTokens.Density.Compact.rowDp
    }
    val avatarDp: Int get() = when (this) {
        COMFORTABLE -> CroftTokens.Density.Comfortable.avatarDp
        DEFAULT -> CroftTokens.Density.Default.avatarDp
        COMPACT -> CroftTokens.Density.Compact.avatarDp
    }
    val previewLines: Int get() = when (this) {
        COMFORTABLE -> CroftTokens.Density.Comfortable.previewLines
        DEFAULT -> CroftTokens.Density.Default.previewLines
        COMPACT -> CroftTokens.Density.Compact.previewLines
    }
}

enum class Roundness(override val id: String) : StoredOption {
    TIGHT("tight"),
    STANDARD("standard"),
    ROUND("round");

    /** Corner scale from the shape tokens: 0.6, 1 and 1.4. */
    val scale: Float get() = when (this) {
        TIGHT -> CroftTokens.Shape.RoundnessScale.tight.toFloat()
        STANDARD -> CroftTokens.Shape.RoundnessScale.standard.toFloat()
        ROUND -> CroftTokens.Shape.RoundnessScale.round.toFloat()
    }

    /** The swipe card corner for this roundness, from the swipe tokens (10, 17 and 24 dp). */
    val swipeCardRadiusDp: Int get() = when (this) {
        TIGHT -> CroftTokens.Swipe.CardRadiusDp.tight
        STANDARD -> CroftTokens.Swipe.CardRadiusDp.standard
        ROUND -> CroftTokens.Swipe.CardRadiusDp.round
    }
}

enum class ListStyle(override val id: String) : StoredOption { CONTAINER("container"), FLAT("flat") }
enum class AvatarStyle(override val id: String) : StoredOption { LETTERS("letters"), PHOTOS("photos"), HIDDEN("hidden") }
enum class UnreadEmphasis(override val id: String) : StoredOption { BOLD_DOT("boldDot"), BOLD("bold"), DOT("dot") }
enum class UiFont(override val id: String) : StoredOption { SYSTEM("system"), CROFT_SANS("croftSans"), ATKINSON("atkinson") }
enum class ReadingFont(override val id: String) : StoredOption { SYSTEM("system"), SERIF("serif"), ATKINSON("atkinson") }

/** Animation level on S53 (docs/ux/motion.md). */
enum class AnimationLevel(override val id: String) : StoredOption { SYSTEM("system"), FULL("full"), REDUCED("reduced"), OFF("off") }

/** Haptics level on S53 (docs/ux/haptics.md); the ids are the tokens in `CroftTokens.Haptics.levels`. */
enum class HapticsLevel(override val id: String) : StoredOption { OFF("off"), SUBTLE("subtle"), FULL("full") }

/**
 * Everything the person chose about the look and feel, equal to the stored shape in docs/ux/theme.md.
 * Immutable; every change is a `copy`. `seed` is an opaque ARGB value (0xFFRRGGBB) like [CroftPreset.seed].
 */
data class ThemeSettings(
    val mode: ModePreference = ModePreference.SYSTEM,
    val pureBlack: Boolean = false,
    val source: ColourSource = ColourSource.PRESET,
    val preset: CroftPreset = CroftPreset.DEFAULT,
    val seed: Long = CroftPreset.DEFAULT.seed,
    val style: ThemeStyle = ThemeStyle.TONAL_SPOT,
    val contrast: CroftContrast = CroftContrast.STANDARD,
    val density: DensityLevel = DensityLevel.DEFAULT,
    val roundness: Roundness = Roundness.STANDARD,
    val listStyle: ListStyle = ListStyle.CONTAINER,
    val avatars: AvatarStyle = AvatarStyle.LETTERS,
    val unreadEmphasis: UnreadEmphasis = UnreadEmphasis.BOLD_DOT,
    val textScale: Float = 1.0f,
    val uiFont: UiFont = UiFont.SYSTEM,
    val readingFont: ReadingFont = ReadingFont.SYSTEM,
    val animation: AnimationLevel = AnimationLevel.SYSTEM,
    val haptics: HapticsLevel = HapticsLevel.SUBTLE,
    /** Per-account colour (avatar stripe, status chip, From chip), keyed by account id. Not a theme colour. */
    val accountColors: Map<String, Long> = emptyMap(),
) {
    /** The seed the colour engine uses: the palette's own seed for a preset, the chosen seed otherwise. */
    val effectiveSeed: Long get() = if (source == ColourSource.CUSTOM) seed else preset.seed

    /** The same settings with a palette applied: the source becomes the preset and the seed follows it. */
    fun withPreset(p: CroftPreset): ThemeSettings = copy(source = ColourSource.PRESET, preset = p, seed = p.seed)

    /** The same settings with a custom seed applied. */
    fun withCustomSeed(argb: Long): ThemeSettings = copy(source = ColourSource.CUSTOM, seed = argb or OPAQUE)

    /** Only the colour choices (what a palette change touches), for the Undo snackbar and tests. */
    fun sameColourChoice(other: ThemeSettings): Boolean =
        source == other.source && preset == other.preset && seed == other.seed && style == other.style &&
            contrast == other.contrast && mode == other.mode && pureBlack == other.pureBlack

    companion object {
        /** Moss, Standard, System: also what the app starts with when the stored file cannot be read. */
        val Default = ThemeSettings()

        /** Text size range on S52 (85 to 150 percent) and the slider step. */
        const val TEXT_SCALE_MIN = 0.85f
        const val TEXT_SCALE_MAX = 1.5f
        const val TEXT_SCALE_STEP = 0.05f

        /** Round to the nearest whole percent and keep inside the range, so a value always survives JSON. */
        fun clampTextScale(v: Float): Float =
            (Math.round(v.coerceIn(TEXT_SCALE_MIN, TEXT_SCALE_MAX) * 100f) / 100f).coerceIn(TEXT_SCALE_MIN, TEXT_SCALE_MAX)

        internal const val OPAQUE = 0xFF000000L
    }
}
