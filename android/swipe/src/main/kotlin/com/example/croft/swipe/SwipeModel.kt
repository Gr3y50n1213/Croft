// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.swipe

/**
 * What a swipe can do (docs/ux/swipe.md, "Available actions"). The [id] is what the settings file stores.
 *
 * [removesRow]: the row leaves the list, so the commit sequence slides it out and collapses its slot (Archive, Delete, Spam,
 * Move to, Snooze). The others (read, star) spring the card back and change the row in place. [parks]: the action needs
 * more from the person (a sheet), so the card is held while it is open. [selectable]: whether S48 offers it; Snooze is
 * phase 3, so the machine supports it but the picker leaves it out until its sheet exists. Move to has its sheet since T16 (S30).
 */
enum class SwipeAction(val id: String, val removesRow: Boolean, val parks: Boolean, val selectable: Boolean = true) {
    /** The direction does nothing; the row does not move at all. */
    NONE("none", removesRow = false, parks = false),

    /** Mark read, or unread when the row is read. */
    TOGGLE_READ("read", removesRow = false, parks = false),

    /** Move to Trash (in Trash and Spam: delete forever, which parks the card behind a confirmation). */
    DELETE("delete", removesRow = true, parks = false),

    /** Move to Archive. */
    ARCHIVE("archive", removesRow = true, parks = false),

    /** Opens the snooze sheet (S29). Not offered until Snooze exists. */
    SNOOZE("snooze", removesRow = true, parks = true, selectable = false),

    /** Opens the folder picker (S30, built in T16). */
    MOVE_TO("move", removesRow = true, parks = true),

    /** Star or unstar. */
    TOGGLE_STAR("star", removesRow = false, parks = false),

    /** Move to Spam. */
    SPAM("spam", removesRow = true, parks = false),
    ;

    companion object {
        /** The action with this stored id, or null. */
        fun fromId(id: String?): SwipeAction? = entries.firstOrNull { it.id == id }

        /** The actions S48 lists, in its order. */
        val picker: List<SwipeAction> = listOf(TOGGLE_READ, ARCHIVE, DELETE, SNOOZE, MOVE_TO, TOGGLE_STAR, SPAM, NONE).filter { it.selectable }
    }
}

/** How far a drag must go before the action arms: Low 45, Normal 35, High 25 percent of the row width (S47). */
enum class SwipeSensitivity(val id: String) {
    /** The long pull: 45 percent. */
    LOW("low"),

    /** The default: 35 percent. */
    NORMAL("normal"),

    /** The short pull: 25 percent. */
    HIGH("high"),
    ;

    companion object {
        /** The sensitivity with this stored id, or null. */
        fun fromId(id: String?): SwipeSensitivity? = entries.firstOrNull { it.id == id }
    }
}

/**
 * A direction of the drag as it is stored: from the start edge toward the end edge, or back. In a left-to-right layout
 * [START_TO_END] is a drag to the right; in right-to-left it is a drag to the left, so one stored setting mirrors by itself.
 */
enum class SwipeDirection {
    /** The drag moves toward the end edge (right in LTR). Settings call it "Swipe right". */
    START_TO_END,

    /** The drag moves toward the start edge (left in LTR). Settings call it "Swipe left". */
    END_TO_START,
    ;

    /** +1 when this direction is a drag to the right on screen, -1 when it is a drag to the left. */
    fun physicalSign(rtl: Boolean): Int = if ((this == START_TO_END) != rtl) 1 else -1

    companion object {
        /** The direction of a physical drag by [dx] (positive is to the right); [dx] must not be zero. */
        fun of(dx: Float, rtl: Boolean): SwipeDirection = if ((dx > 0f) != rtl) START_TO_END else END_TO_START
    }
}

/** What the person chose in S47: one action per direction and the sensitivity. */
data class SwipeSettings(
    val startToEnd: SwipeAction = SwipeAction.TOGGLE_READ,
    val endToStart: SwipeAction = SwipeAction.DELETE,
    val sensitivity: SwipeSensitivity = SwipeSensitivity.NORMAL,
) {
    /** The action of [direction]. */
    fun actionOf(direction: SwipeDirection): SwipeAction = if (direction == SwipeDirection.START_TO_END) startToEnd else endToStart

    /** The settings with [action] for [direction]. */
    fun with(direction: SwipeDirection, action: SwipeAction): SwipeSettings =
        if (direction == SwipeDirection.START_TO_END) copy(startToEnd = action) else copy(endToStart = action)

    /** The preset these settings equal (sensitivity is not part of a preset), or null. */
    fun matchingPreset(): SwipePreset? = SwipePreset.entries.firstOrNull { it.startToEnd == startToEnd && it.endToStart == endToStart }

    companion object {
        /** Triage: the default. */
        val Default = SwipeSettings()
    }
}

/**
 * The presets of S47. "Inbox zero" (right Archive, left Snooze) is not here: Snooze is phase 3 and the picker leaves it
 * out until it exists, so the preset would set something that cannot work.
 */
enum class SwipePreset(val id: String, val startToEnd: SwipeAction, val endToStart: SwipeAction) {
    /** Right Mark read, left Delete. The default. */
    TRIAGE("triage", SwipeAction.TOGGLE_READ, SwipeAction.DELETE),

    /** Both Archive. */
    GMAIL_STYLE("gmail", SwipeAction.ARCHIVE, SwipeAction.ARCHIVE),

    /** Right Mark read, left Archive. */
    TIDY("tidy", SwipeAction.TOGGLE_READ, SwipeAction.ARCHIVE),
}
