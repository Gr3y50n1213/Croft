// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.swipe

/** The geometry numbers of docs/ux/swipe.md; the app reads them from `tokens.json` (`swipe`). All lengths in dp. */
data class SwipeGeometryConfig(
    /** The gap between the card and the pill. */
    val gapToCard: Float,
    /** The gap between the pill and the row's outer edge. */
    val insetFromEdge: Float,
    /** The pill is as wide as the drag distance minus this (the gap plus the inset). */
    val widthIsDragMinus: Float,
    /** The card's corners ease from the row's own to the card radius over this much drag. */
    val cardEaseInDistance: Float,
    /** The icon is drawn once the pill is at least this wide. */
    val iconSize: Float,
)

/** The pill that fills the space the card leaves, for one drag distance. */
data class PillGeometry(
    /** Width: the drag distance minus 12 dp, never negative. */
    val width: Float,
    /** Height: the row's. */
    val height: Float,
    /** Corner radius: half the smaller side, so it starts as a capsule and widens into a stadium. */
    val radius: Float,
    /** Space between the pill and the card. */
    val gapToCard: Float,
    /** Space between the pill and the row's outer edge. */
    val insetFromEdge: Float,
    /** Whether the icon is drawn (the pill is wide enough to hold it). */
    val iconVisible: Boolean,
)

/** Pure geometry of the swipe visuals, so the numbers can be checked without drawing anything. */
object SwipeGeometry {
    /** The pill for a card [offset] (how far the card has moved, in dp) on a row [rowHeight] high. */
    fun pill(cfg: SwipeGeometryConfig, offset: Float, rowHeight: Float): PillGeometry {
        val width = maxOf(0f, offset - cfg.widthIsDragMinus)
        return PillGeometry(
            width = width,
            height = rowHeight,
            radius = minOf(width, rowHeight) / 2f,
            gapToCard = cfg.gapToCard,
            insetFromEdge = cfg.insetFromEdge,
            iconVisible = width >= cfg.iconSize,
        )
    }

    /** The pill when the commit has widened it: the whole row less the inset on both sides. */
    fun committedPill(cfg: SwipeGeometryConfig, rowWidth: Float, rowHeight: Float): PillGeometry {
        val width = maxOf(0f, rowWidth - 2f * cfg.insetFromEdge)
        return PillGeometry(width, rowHeight, minOf(width, rowHeight) / 2f, cfg.gapToCard, cfg.insetFromEdge, width >= cfg.iconSize)
    }

    /** The card's corner radius at a card [offset]: from [restRadius] (the row's own corners) to [cardRadius] over the first 16 dp. */
    fun cardRadius(cfg: SwipeGeometryConfig, offset: Float, restRadius: Float, cardRadius: Float): Float {
        val f = if (cfg.cardEaseInDistance <= 0f) 1f else (offset / cfg.cardEaseInDistance).coerceIn(0f, 1f)
        return restRadius + (cardRadius - restRadius) * f
    }
}
