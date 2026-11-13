// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.ui.unit.dp
import com.example.croft.design.CroftTokens

/** Test tags of the message list screens (S10 to S19). */
object InboxTags {
    const val SCREEN = "inbox"
    const val LIST = "inbox-list"
    const val SKELETON = "inbox-skeleton"
    const val SKELETON_ROW = "inbox-skeleton-row"
    const val EMPTY = "inbox-empty"
    const val SEARCH_PILL = "inbox-search-pill"
    const val MENU_BUTTON = "inbox-menu"
    const val ACCOUNT_BUTTON = "inbox-account"
    const val FAB = "inbox-fab"
    const val NEW_PILL = "inbox-new-pill"
    const val SELECTION_BAR = "inbox-selection-bar"
    const val SELECT_ALL = "inbox-select-all"
    const val SELECT_ALL_IN_FOLDER = "inbox-select-all-in-folder"
    const val BANNER = "inbox-banner"
    const val BOTTOM_BAR = "inbox-bottom-bar"
    const val DRAWER = "inbox-drawer"
    const val SWITCHER = "inbox-switcher"
    const val PULL_INDICATOR = "inbox-pull-indicator"
    const val FAST_SCROLLER = "inbox-fast-scroller"
    const val CHIPS = "inbox-chips"

    /** The chip of category [c] (T35). */
    fun chip(c: com.example.croft.mail.MailCategory) = "inbox-chip-${c.key}"

    /** The tag of the row of message [id]. */
    fun row(id: Long) = "row-$id"

    /** The tag of the date header with the key [bucketId] (see `DateBucket.id`). */
    fun header(bucketId: String) = "header-$bucketId"

    /** The tag of a drawer entry. */
    fun drawerItem(folderId: Long) = "drawer-folder-$folderId"

    /** The tag of the long-press menu of a drawer entry. */
    fun drawerMenu(folderId: Long) = "drawer-menu-$folderId"

    /** The tag of an account row of the switcher; [id] 0 is "All accounts". */
    fun switcherAccount(id: Long) = "switcher-account-$id"
}

/** The pull to refresh arming distance, from docs/ux/state-machines.md ("Drag past 80 dp"); `tokens.json` `layout.pullToRefreshArmDp`. */
val PULL_ARM_DP = CroftTokens.Layout.pullToRefreshArmDp.dp

/** The widths at which the layout changes (Material window size classes): the drawer is permanent from [EXPANDED_FROM_DP]. */
const val EXPANDED_FROM_DP = CroftTokens.Layout.expandedWidthFromDp
