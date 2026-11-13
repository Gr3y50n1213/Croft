// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.contacts

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.Haptics
import com.example.croft.haptics.LocalHaptics
import kotlin.math.roundToInt

/** Test tags of the A to Z index. */
object AlphabetIndexTags {
    const val STRIP = "alphabet-index"
    const val BUBBLE = "alphabet-index-bubble"
}

/**
 * S40's A to Z index: a strip of letters on the trailing edge. A tap or a drag over it picks the letter under the finger,
 * [onLetter] is called with it, the Tick haptic plays at each new letter, and a bubble with the letter shows beside the finger
 * while it is down. Letters without a section ([present]) are drawn faded but still pick (the list goes to the nearest section).
 * It is for touch and mouse only: screen readers scroll the list itself, so it is hidden from them. The letters are sized to fit
 * their slot whatever the font scale, so all of them always show. Nothing animates: the bubble appears and goes at once.
 */
@Composable
fun AlphabetIndex(
    letters: List<String>,
    present: Set<String>,
    onLetter: (String) -> Unit,
    modifier: Modifier = Modifier,
    haptics: Haptics = LocalHaptics.current,
    /** The gallery: show the bubble at this letter without a finger. */
    forceBubble: String? = null,
) {
    if (letters.isEmpty()) return
    var heightPx by remember { mutableIntStateOf(0) }
    var active by remember { mutableStateOf<String?>(null) }
    var fingerY by remember { mutableFloatStateOf(0f) }
    val pick by rememberUpdatedState(onLetter)
    val density = LocalDensity.current
    val slotPx = if (heightPx > 0) heightPx.toFloat() / letters.size else 0f
    // Fit the letter to its slot, whatever the font scale (the index is a gesture surface, hidden from screen readers).
    val fontSize = with(density) { (slotPx * LETTER_FILL).coerceIn(0f, MAX_LETTER.toPx()).toSp() }
    val bubble = forceBubble ?: active
    val bubbleY = if (forceBubble != null) (letters.indexOf(forceBubble).coerceAtLeast(0) + 0.5f) * slotPx else fingerY
    Box(modifier.fillMaxHeight().width(STRIP_WIDTH)) {
        Column(
            Modifier
                .fillMaxHeight()
                .width(STRIP_WIDTH)
                .onSizeChanged { heightPx = it.height }
                .testTag(AlphabetIndexTags.STRIP)
                .clearAndSetSemantics { }
                .pointerInput(letters) {
                    fun at(y: Float): String = letters[((y / size.height) * letters.size).toInt().coerceIn(0, letters.size - 1)]
                    awaitEachGesture {
                        val down = awaitFirstDown()
                        down.consume()
                        fingerY = down.position.y.coerceIn(0f, size.height.toFloat())
                        var last = at(down.position.y)
                        active = last
                        haptics.perform(HapticEvent.Tick)
                        pick(last)
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (!change.pressed) break
                            change.consume()
                            fingerY = change.position.y.coerceIn(0f, size.height.toFloat())
                            val now = at(change.position.y)
                            if (now != last) {
                                last = now
                                active = now
                                haptics.perform(HapticEvent.Tick)
                                pick(now)
                            }
                        }
                        active = null
                    }
                },
        ) {
            for (l in letters) {
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    Text(
                        l,
                        fontSize = fontSize,
                        lineHeight = fontSize,
                        maxLines = 1,
                        textAlign = TextAlign.Center,
                        fontWeight = if (l == bubble) FontWeight.Bold else FontWeight.Medium,
                        color = if (l in present) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = ABSENT_ALPHA),
                    )
                }
            }
        }
        if (bubble != null && heightPx > 0) {
            val bubblePx = with(density) { BUBBLE_SIZE.toPx() }
            Surface(
                Modifier
                    .offset { IntOffset(with(density) { -(BUBBLE_GAP + BUBBLE_SIZE).roundToPx() }, (bubbleY - bubblePx / 2).roundToInt().coerceIn(0, (heightPx - bubblePx).roundToInt().coerceAtLeast(0))) }
                    .size(BUBBLE_SIZE)
                    .testTag(AlphabetIndexTags.BUBBLE)
                    .clearAndSetSemantics { },
                shape = MaterialTheme.shapes.extraLarge,
                color = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(bubble, style = MaterialTheme.typography.headlineSmall, maxLines = 1, fontSize = with(density) { (BUBBLE_SIZE * BUBBLE_FILL).toSp() })
                }
            }
        }
    }
}

/** The section to show for [letter]: itself when it has one, else the next one after it, else the last one before it. */
fun nearestSection(letter: String, letters: List<String>, present: Set<String>): String? {
    val i = letters.indexOf(letter)
    if (i < 0) return null
    return letters.drop(i).firstOrNull { it in present } ?: letters.take(i).lastOrNull { it in present }
}

private val STRIP_WIDTH = 24.dp
private val MAX_LETTER = 12.dp
private const val LETTER_FILL = 0.8f
private val BUBBLE_SIZE = 56.dp
private val BUBBLE_GAP = 8.dp
private const val BUBBLE_FILL = 0.45f
private const val ABSENT_ALPHA = 0.5f
