// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.foundation.shape.CornerBasedShape
import androidx.compose.foundation.shape.CornerSize
import androidx.compose.material3.Shapes
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.Density

/**
 * A corner radius scaled by the Roundness setting (0.6, 1 or 1.4 from the shape tokens) and never larger than half
 * the component's height, so a chip or a button cannot be rounder than a pill.
 */
internal class ScaledCornerSize(private val base: CornerSize, private val scale: Float) : CornerSize {
    override fun toPx(shapeSize: Size, density: Density): Float =
        minOf(base.toPx(shapeSize, density) * scale, shapeSize.height / 2f)

    override fun equals(other: Any?): Boolean = other is ScaledCornerSize && other.base == base && other.scale == scale
    override fun hashCode(): Int = 31 * base.hashCode() + scale.hashCode()
    override fun toString(): String = "ScaledCornerSize($base x $scale)"
}

private fun Shape.scaled(scale: Float): Shape =
    if (this is CornerBasedShape && scale != 1f) {
        copy(
            topStart = ScaledCornerSize(topStart, scale),
            topEnd = ScaledCornerSize(topEnd, scale),
            bottomEnd = ScaledCornerSize(bottomEnd, scale),
            bottomStart = ScaledCornerSize(bottomStart, scale),
        )
    } else this

/** Every Material shape with its corners scaled; a scale of 1 returns the shapes unchanged. */
internal fun Shapes.scaledBy(scale: Float): Shapes = copy(
    extraSmall = extraSmall.scaled(scale) as CornerBasedShape,
    small = small.scaled(scale) as CornerBasedShape,
    medium = medium.scaled(scale) as CornerBasedShape,
    large = large.scaled(scale) as CornerBasedShape,
    extraLarge = extraLarge.scaled(scale) as CornerBasedShape,
)
