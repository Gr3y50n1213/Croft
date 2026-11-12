// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The swipe visuals' geometry (docs/ux/swipe.md, "The pill"), ported from SwipeGeometry.kt. Lengths in px (= dp). */

export interface SwipeGeometryConfig {
  /** The gap between the card and the pill. */
  readonly gapToCard: number;
  /** The gap between the pill and the row's outer edge. */
  readonly insetFromEdge: number;
  /** The pill is as wide as the drag distance minus this. */
  readonly widthIsDragMinus: number;
  /** The card's corners ease from the row's own to the card radius over this much drag. */
  readonly cardEaseInDistance: number;
  /** The icon is drawn once the pill is at least this wide. */
  readonly iconSize: number;
}

export interface PillGeometry {
  readonly width: number;
  readonly height: number;
  readonly radius: number;
  readonly gapToCard: number;
  readonly insetFromEdge: number;
  readonly iconVisible: boolean;
}

/** The pill for a card `offset` on a row `rowHeight` high. */
export function pill(cfg: SwipeGeometryConfig, offset: number, rowHeight: number): PillGeometry {
  const width = Math.max(0, offset - cfg.widthIsDragMinus);
  return {
    width,
    height: rowHeight,
    radius: Math.min(width, rowHeight) / 2,
    gapToCard: cfg.gapToCard,
    insetFromEdge: cfg.insetFromEdge,
    iconVisible: width >= cfg.iconSize
  };
}

/** The pill when the commit has widened it: the whole row less the inset on both sides. */
export function committedPill(cfg: SwipeGeometryConfig, rowWidth: number, rowHeight: number): PillGeometry {
  const width = Math.max(0, rowWidth - 2 * cfg.insetFromEdge);
  return {
    width,
    height: rowHeight,
    radius: Math.min(width, rowHeight) / 2,
    gapToCard: cfg.gapToCard,
    insetFromEdge: cfg.insetFromEdge,
    iconVisible: width >= cfg.iconSize
  };
}

/** The card's corner radius at a card `offset`: from `restRadius` to `cardRadius` over the first 16 px. */
export function cardRadius(cfg: SwipeGeometryConfig, offset: number, restRadius: number, cardR: number): number {
  const f = cfg.cardEaseInDistance <= 0 ? 1 : Math.min(1, Math.max(0, offset / cfg.cardEaseInDistance));
  return restRadius + (cardR - restRadius) * f;
}
