// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.swipe

/** The numbers of docs/ux/swipe.md, written out for the tests (the app reads the same ones from `tokens.json`). */
val TestTuning = SwipeTuning(
    armLow = 0.45f, armNormal = 0.35f, armHigh = 0.25f,
    flingMinVelocity = 1200f, flingMinTravelFraction = 0.2f,
    parkFraction = 0.4f, resistanceStartFraction = 0.85f, resistanceFactor = 0.3333f,
    crossingGuardMs = 40,
)

val TestGeometry = SwipeGeometryConfig(gapToCard = 4f, insetFromEdge = 8f, widthIsDragMinus = 12f, cardEaseInDistance = 16f, iconSize = 24f)

const val W = 400f

fun ctx(
    start: SwipeSlot = SwipeSlot(SwipeAction.TOGGLE_READ),
    end: SwipeSlot = SwipeSlot(SwipeAction.DELETE),
    rtl: Boolean = false,
    sensitivity: SwipeSensitivity = SwipeSensitivity.NORMAL,
    width: Float = W,
) = SwipeContext(width, rtl, sensitivity, start, end)

fun machine(c: SwipeContext = ctx()) = SwipeMachine(TestTuning, c)

fun SwipeMachine.drag(dx: Float, t: Long) = send(SwipeInput.Drag(dx, t))
fun SwipeMachine.release(v: Float = 0f, t: Long = 10_000) = send(SwipeInput.Release(v, t))

fun List<SwipeEffect>.haptics(): List<SwipeHaptic> = filterIsInstance<SwipeEffect.Haptic>().map { it.kind }
