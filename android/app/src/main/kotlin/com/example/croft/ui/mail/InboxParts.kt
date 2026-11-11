// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.snap
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Create
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.Haptics
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import com.example.croft.mail.FolderRole
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.LocalThemeSettings
import com.example.croft.theme.MotionLevel
import com.example.croft.theme.shimmerSpec
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftIcons
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.CroftSpinner
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.MenuItemSpec
import com.example.croft.ui.components.minTouchTarget
import com.example.croft.ui.components.motionProbe
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.roundToInt

// ---------------------------------------------------------------------------------------------------------------------
// The top of S10: search pill, selection bar (S11), category chips

/**
 * The search pill: menu (opens the drawer), "Search in mail" (search is S27, later) and the account avatar (opens the account
 * switcher, S13). On wide windows the drawer is always there, so [showMenu] is false and the leading icon is a magnifier.
 */
@Composable
fun SearchPill(
    accountLabel: String,
    accountStatusText: String?,
    onMenu: () -> Unit,
    onSearch: () -> Unit,
    onAccount: () -> Unit,
    modifier: Modifier = Modifier,
    showMenu: Boolean = true,
) {
    Surface(modifier.fillMaxWidth().testTag(InboxTags.SEARCH_PILL), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHigh) {
        Row(Modifier.heightIn(min = 56.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            if (showMenu) {
                CroftIconButton(Icons.Filled.Menu, stringResource(R.string.mail_open_menu), onMenu, Modifier.testTag(InboxTags.MENU_BUTTON))
            } else {
                Icon(Icons.Filled.Search, contentDescription = null, Modifier.padding(horizontal = 12.dp))
            }
            Box(Modifier.weight(1f).heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onSearch).padding(horizontal = 8.dp), contentAlignment = Alignment.CenterStart) {
                Text(
                    stringResource(R.string.mail_search_in_mail),
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            AccountAvatarButton(accountLabel, accountStatusText, onAccount)
        }
    }
}

/** The round account avatar: the first letter, with the account's status read after its name when it needs attention. */
@Composable
fun AccountAvatarButton(accountLabel: String, statusText: String?, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val description = if (statusText == null) stringResource(R.string.mail_account_button, accountLabel) else stringResource(R.string.mail_account_button_status, accountLabel, statusText)
    Box(
        modifier.testTag(InboxTags.ACCOUNT_BUTTON).minTouchTarget().clickable(role = Role.Button, onClick = onClick).semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) { InitialCircle(accountLabel, 40.dp, MaterialTheme.colorScheme.primary, MaterialTheme.colorScheme.onPrimary) }
}

/** A circle with the first letter of [label]. The circle has a fixed size, so the letter does not grow with the text size. */
@Composable
fun InitialCircle(label: String, size: androidx.compose.ui.unit.Dp, container: androidx.compose.ui.graphics.Color, content: androidx.compose.ui.graphics.Color, modifier: Modifier = Modifier) {
    val outer = LocalDensity.current
    Box(modifier.size(size).clip(CircleShape).background(container).clearAndSetSemantics { }, contentAlignment = Alignment.Center) {
        androidx.compose.runtime.CompositionLocalProvider(LocalDensity provides Density(outer.density, fontScale = minOf(outer.fontScale, 1f))) {
            Text(label.trim().firstOrNull()?.uppercaseChar()?.toString() ?: "?", style = MaterialTheme.typography.titleMedium, color = content)
        }
    }
}

/** The category chips of S10. Only "All" until the smart-inbox phase; the row is a component so the others can join it. */
@Composable
fun CategoryChipsRow(modifier: Modifier = Modifier, categories: List<String> = listOf(stringResource(R.string.mail_category_all)), selected: Int = 0, onSelect: (Int) -> Unit = {}) {
    Row(
        modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag(InboxTags.CHIPS),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        categories.forEachIndexed { i, name -> CroftFilterChip(name, selected = i == selected, onClick = { onSelect(i) }) }
    }
}

/** The icon of a selection action. */
fun SelectionAction.icon(): ImageVector = when (this) {
    SelectionAction.ARCHIVE -> CroftIcons.Archive
    SelectionAction.DELETE, SelectionAction.DELETE_FOREVER -> Icons.Filled.Delete
    SelectionAction.MARK_READ -> CroftIcons.EnvelopeOpen
    SelectionAction.MARK_UNREAD -> Icons.Filled.Email
    SelectionAction.STAR, SelectionAction.UNSTAR -> Icons.Filled.Star
    SelectionAction.RESTORE -> CroftIcons.Inbox
}

/** The words of a selection action; Restore depends on where the messages are (Move to inbox, Not spam, Restore). */
@Composable
fun SelectionAction.label(role: FolderRole): String = stringResource(
    when (this) {
        SelectionAction.ARCHIVE -> R.string.mail_action_archive
        SelectionAction.DELETE -> R.string.mail_action_delete
        SelectionAction.DELETE_FOREVER -> R.string.mail_action_delete_forever
        SelectionAction.MARK_READ -> R.string.mail_action_mark_read
        SelectionAction.MARK_UNREAD -> R.string.mail_action_mark_unread
        SelectionAction.STAR -> R.string.mail_action_star
        SelectionAction.UNSTAR -> R.string.mail_action_unstar
        SelectionAction.RESTORE -> when (role) {
            FolderRole.SPAM -> R.string.mail_action_not_spam
            FolderRole.TRASH -> R.string.mail_action_restore
            else -> R.string.mail_action_move_to_inbox
        }
    },
)

/**
 * The contextual bar of selection mode (S11): close, the count, the actions that apply to everything selected, and an overflow
 * menu. At a large text size the count takes its own line above the buttons, so nothing is cut. Below it, "Select all" and,
 * when more messages exist than are loaded, "Select all N in this folder".
 */
@Composable
fun SelectionBar(
    count: Int,
    role: FolderRole,
    toolbar: SelectionToolbar,
    onClose: () -> Unit,
    onAction: (SelectionAction) -> Unit,
    offerSelectAllInFolder: Boolean,
    totalInFolder: Int,
    allInFolderSelected: Boolean,
    onSelectAll: () -> Unit,
    onSelectAllInFolder: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var menuOpen by remember { mutableStateOf(false) }
    val bigText = LocalDensity.current.fontScale > BAR_STACK_FROM_FONT_SCALE
    val countText = if (allInFolderSelected) stringResource(R.string.mail_selection_all_in_folder, "%,d".format(totalInFolder)) else pluralStringResource(R.plurals.mail_selected_count, count, count)
    val close: @Composable () -> Unit = { CroftIconButton(Icons.Filled.Close, stringResource(R.string.mail_selection_exit), onClose) }
    val countLabel: @Composable (Modifier) -> Unit = { m ->
        Text(countText, m.semantics { liveRegion = LiveRegionMode.Polite }, style = MaterialTheme.typography.titleMedium, maxLines = if (bigText) 2 else 1, overflow = TextOverflow.Ellipsis)
    }
    val primaryLabels = toolbar.primary.map { it.label(role) }
    val overflowItems = toolbar.overflow.map { a -> MenuItemSpec(label = a.label(role), onClick = { onAction(a) }, icon = a.icon()) }
    val buttons: @Composable () -> Unit = {
        toolbar.primary.forEachIndexed { i, action -> CroftIconButton(action.icon(), primaryLabels[i], { onAction(action) }, Modifier.testTag("selection-action-${action.name}")) }
        Box {
            CroftIconButton(Icons.Filled.MoreVert, stringResource(R.string.mail_more_actions), { menuOpen = true })
            CroftMenu(menuOpen, { menuOpen = false }, overflowItems)
        }
    }
    Column(modifier.fillMaxWidth().testTag(InboxTags.SELECTION_BAR)) {
        Surface(shape = CircleShape.takeIf { !bigText } ?: MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.secondaryContainer, contentColor = MaterialTheme.colorScheme.onSecondaryContainer) {
            if (bigText) {
                Column(Modifier.padding(horizontal = 4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) { close(); countLabel(Modifier.weight(1f)) }
                    Row(Modifier.align(Alignment.End), verticalAlignment = Alignment.CenterVertically) { buttons() }
                }
            } else {
                Row(Modifier.heightIn(min = 56.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    close(); countLabel(Modifier.weight(1f)); buttons()
                }
            }
        }
        Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            androidx.compose.material3.AssistChip(
                onClick = onSelectAll,
                label = { Text(stringResource(R.string.mail_select_all), maxLines = 1) },
                modifier = Modifier.minTouchTarget().testTag(InboxTags.SELECT_ALL),
            )
            if (offerSelectAllInFolder) {
                androidx.compose.material3.AssistChip(
                    onClick = onSelectAllInFolder,
                    label = { Text(stringResource(R.string.mail_select_all_in_folder, "%,d".format(totalInFolder)), maxLines = 2) },
                    modifier = Modifier.minTouchTarget().testTag(InboxTags.SELECT_ALL_IN_FOLDER),
                )
            }
        }
    }
}

/** From this combined font scale the selection bar stacks its count above its buttons. */
private const val BAR_STACK_FROM_FONT_SCALE = 1.3f

// ---------------------------------------------------------------------------------------------------------------------
// New mail pill (S17)

/**
 * The "N new" pill. It appears only when new mail arrived above the place the person is reading; the list never moves. Tapping
 * it goes to the top. It is a polite live region ("3 new messages") and keeps accepting touches while it moves.
 * It rises with the spatial default spring at Full, fades without travel at Reduced, and is there at once at Off. If it is
 * hidden and shown again mid-way the spring carries on from where it is (it is one animated value, never restarted).
 */
@Composable
fun NewMailPill(count: Int, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val level = LocalMotionLevel.current
    val scheme = MaterialTheme.motionScheme
    val visible = count > 0
    val spec = when (level) {
        MotionLevel.FULL -> scheme.defaultSpatialSpec<Float>()
        MotionLevel.REDUCED -> scheme.defaultEffectsSpec<Float>()
        MotionLevel.OFF -> snap<Float>()
    }
    val progress by animateFloatAsState(if (visible) 1f else 0f, spec, label = "new-mail-pill")
    var shownCount by remember { mutableIntStateOf(count) }
    if (count > 0) shownCount = count
    if (progress <= 0f && !visible) return
    val rise = with(LocalDensity.current) { PILL_RISE.toPx() }
    val description = pluralStringResource(R.plurals.mail_new_pill_description, shownCount, shownCount)
    Surface(
        onClick = onClick,
        enabled = visible,
        modifier = modifier
            .testTag(InboxTags.NEW_PILL)
            .motionProbe("pill") { progress }
            .graphicsLayer {
                alpha = progress.coerceIn(0f, 1f)
                translationY = if (level == MotionLevel.FULL) (1f - progress) * rise else 0f
            }
            .semantics(mergeDescendants = true) { contentDescription = description; liveRegion = LiveRegionMode.Polite },
        shape = CircleShape,
        color = MaterialTheme.colorScheme.primary,
        contentColor = MaterialTheme.colorScheme.onPrimary,
        shadowElevation = 3.dp,
    ) {
        Row(Modifier.heightIn(min = 48.dp).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Filled.KeyboardArrowUp, contentDescription = null)
            Text(pluralStringResource(R.plurals.mail_new_pill, shownCount, shownCount), style = MaterialTheme.typography.labelLarge)
        }
    }
}

private val PILL_RISE = 16.dp

// ---------------------------------------------------------------------------------------------------------------------
// Pull to refresh (S17)

/**
 * The indicator of pull to refresh: it grows with the drag and morphs at the threshold (Material's expressive loading
 * indicator, driven by the drag fraction), then spins while refreshing. At Reduced and Off it is the static spinner and does
 * not grow (docs/ux/motion.md). The arming haptic is played by [PullRefreshHaptics].
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun PullIndicator(state: PullToRefreshState, refreshing: Boolean, modifier: Modifier = Modifier) {
    val level = LocalMotionLevel.current
    val thresholdPx = with(LocalDensity.current) { PULL_ARM_DP.toPx() }
    val sizePx = with(LocalDensity.current) { INDICATOR_SIZE.toPx() }
    val armed = stringResource(R.string.mail_release_to_refresh)
    val pulling = stringResource(R.string.mail_pull_to_refresh)
    val busy = stringResource(R.string.mail_refreshing)
    val show = state.distanceFraction > 0f || refreshing
    if (!show) return
    Surface(
        modifier
            .testTag(InboxTags.PULL_INDICATOR)
            .motionProbe("pull") { state.distanceFraction }
            .graphicsLayer {
                val f = if (refreshing) 1f else state.distanceFraction
                // Growing with the drag is movement: only at Full. Otherwise it sits where it would end up.
                val place = if (level == MotionLevel.FULL) f.coerceAtMost(PULL_MAX_FRACTION) else 1f
                translationY = place * thresholdPx - sizePx
                alpha = if (level == MotionLevel.FULL) f.coerceIn(0f, 1f) else 1f
            }
            .semantics { contentDescription = if (refreshing) busy else if (state.distanceFraction >= 1f) armed else pulling; if (refreshing) liveRegion = LiveRegionMode.Polite },
        shape = CircleShape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shadowElevation = 2.dp,
    ) {
        Box(Modifier.size(INDICATOR_SIZE).padding(8.dp), contentAlignment = Alignment.Center) {
            if (level == MotionLevel.FULL) {
                if (refreshing) LoadingIndicator() else LoadingIndicator(progress = { state.distanceFraction.coerceIn(0f, 1f) })
            } else CroftSpinner(Modifier.size(24.dp))
        }
    }
}

private val INDICATOR_SIZE = 48.dp
private const val PULL_MAX_FRACTION = 1.5f

/** Plays ThresholdArmed when the pull passes the threshold and ThresholdDisarmed when it goes back (not once refreshing). */
@Composable
fun PullRefreshHaptics(state: PullToRefreshState, refreshing: Boolean, haptics: Haptics) {
    LaunchedEffect(state, haptics) {
        var wasArmed = false
        snapshotFlow { state.distanceFraction >= 1f }.collect { armed ->
            if (armed && !wasArmed && !refreshing) haptics.perform(HapticEvent.ThresholdArmed)
            if (!armed && wasArmed && !refreshing && state.distanceFraction > 0f) haptics.perform(HapticEvent.ThresholdDisarmed)
            wasArmed = armed
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Skeleton rows (first load only)

/**
 * Skeleton rows for the first load of a folder, never for a refresh. A slow shimmer (1.2 s, linear) at Full only
 * (a pulse of the placeholders' opacity); static placeholders when motion is reduced (Reduced and Off), as the catalogue says.
 */
@Composable
fun SkeletonList(modifier: Modifier = Modifier, rows: Int = 10) {
    val level = LocalMotionLevel.current
    val settings = LocalThemeSettings.current
    val phase: Float = if (level == MotionLevel.FULL) {
        val t = rememberInfiniteTransition(label = "skeleton")
        val p by t.animateFloat(0f, 1f, shimmerSpec(), label = "skeleton-phase")
        p
    } else 0f
    val pulse = if (level == MotionLevel.FULL) 0.5f + 0.5f * (1f - abs(2f * phase - 1f)) else 1f
    val bar = MaterialTheme.colorScheme.surfaceContainerHighest
    Column(
        modifier.fillMaxSize().testTag(InboxTags.SKELETON).motionProbe("shimmer") { phase },
    ) {
        val loading = stringResource(R.string.mail_loading)
        Box(Modifier.size(0.dp).semantics { contentDescription = loading; liveRegion = LiveRegionMode.Polite })
        repeat(rows) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = settings.density.rowDp.dp).padding(horizontal = 16.dp, vertical = 12.dp).testTag(InboxTags.SKELETON_ROW),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(Modifier.size(settings.density.avatarDp.dp).graphicsLayer { alpha = pulse }.clip(CircleShape).background(bar))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.fillMaxWidth(0.4f).height(12.dp).graphicsLayer { alpha = pulse }.clip(MaterialTheme.shapes.small).background(bar))
                    Box(Modifier.fillMaxWidth(0.8f).height(12.dp).graphicsLayer { alpha = pulse }.clip(MaterialTheme.shapes.small).background(bar))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Date header, empty state, bottom bar, fast scroller

/** A sticky date header. It is also what the fast scroller shows while it is dragged. */
@Composable
fun DateHeader(label: String, modifier: Modifier = Modifier) {
    Surface(modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.surfaceContainerLow) {
        Text(
            label,
            Modifier.padding(horizontal = 16.dp, vertical = 6.dp).semantics { heading() },
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
        )
    }
}

/** S19: a calm empty state. The illustration slot is a placeholder box (art comes later). */
@Composable
fun EmptyState(kind: EmptyKind, lastSynced: String?, onCompose: () -> Unit, onSearchServer: () -> Unit, modifier: Modifier = Modifier) {
    val title = when (kind) {
        EmptyKind.InboxZero -> stringResource(R.string.mail_empty_inbox_title)
        EmptyKind.Trash -> stringResource(R.string.mail_empty_trash_title)
        EmptyKind.Spam -> stringResource(R.string.mail_empty_spam_title)
        EmptyKind.Folder -> stringResource(R.string.mail_empty_folder_title)
        is EmptyKind.Search -> stringResource(R.string.mail_empty_search_title, kind.query)
    }
    val body = when (kind) {
        EmptyKind.InboxZero -> lastSynced?.let { stringResource(R.string.mail_empty_last_synced, it) } ?: stringResource(R.string.mail_empty_never_synced)
        EmptyKind.Trash -> stringResource(R.string.mail_empty_trash_body)
        EmptyKind.Spam -> stringResource(R.string.mail_empty_spam_body)
        EmptyKind.Folder -> stringResource(R.string.mail_empty_folder_body)
        is EmptyKind.Search -> if (kind.serverToo) null else stringResource(R.string.mail_empty_search_local)
    }
    Column(
        modifier.fillMaxSize().testTag(InboxTags.EMPTY).padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
    ) {
        Box(
            Modifier.size(160.dp).clip(MaterialTheme.shapes.extraLarge).background(MaterialTheme.colorScheme.surfaceContainerHigh),
            contentAlignment = Alignment.Center,
        ) { Icon(CroftIcons.Inbox, contentDescription = null, Modifier.size(64.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
        Text(title, style = MaterialTheme.typography.titleLarge, textAlign = androidx.compose.ui.text.style.TextAlign.Center, modifier = Modifier.semantics { heading() })
        if (body != null) Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        when (kind) {
            EmptyKind.InboxZero -> CroftButton(stringResource(R.string.mail_compose), onCompose, kind = ButtonKind.TONAL, leadingIcon = Icons.Filled.Create)
            is EmptyKind.Search -> if (!kind.serverToo) CroftButton(stringResource(R.string.mail_empty_search_server), onSearchServer, kind = ButtonKind.TONAL)
            else -> Unit
        }
    }
}

/**
 * The bottom bar: Mail, Calendar, Contacts. Calendar and Contacts are later steps, so they are disabled (and still announced).
 * In selection mode the whole bar is disabled. Tapping the active item scrolls the list to the top; tapping it again at
 * the top refreshes.
 */
@Composable
fun MailBottomBar(enabled: Boolean, onMail: () -> Unit, modifier: Modifier = Modifier) {
    NavigationBar(modifier.testTag(InboxTags.BOTTOM_BAR)) {
        NavigationBarItem(selected = true, onClick = onMail, enabled = enabled, icon = { Icon(Icons.Filled.Email, contentDescription = null) }, label = { Text(stringResource(R.string.mail_nav_mail)) })
        NavigationBarItem(selected = false, onClick = {}, enabled = false, icon = { Icon(Icons.Filled.DateRange, contentDescription = null) }, label = { Text(stringResource(R.string.mail_nav_calendar)) })
        NavigationBarItem(selected = false, onClick = {}, enabled = false, icon = { Icon(Icons.Filled.Person, contentDescription = null) }, label = { Text(stringResource(R.string.mail_nav_contacts)) })
    }
}

/**
 * The fast scroller: a thumb on the trailing edge that shows while the list scrolls and can be dragged; while it is dragged a
 * bubble shows the date header of the place (the sticky headers double as its labels) and the Tick haptic plays at each new
 * label. It is for touch and mouse only: screen readers scroll the list itself, so it is hidden from them.
 */
@Composable
fun FastScroller(listState: LazyListState, items: List<ListItem>, labelOf: (DateBucket) -> String, haptics: Haptics, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val level = LocalMotionLevel.current
    var trackPx by remember { mutableIntStateOf(0) }
    var dragging by remember { mutableStateOf(false) }
    var dragFraction by remember { mutableFloatStateOf(0f) }
    var lastLabel by remember { mutableStateOf<String?>(null) }
    val density = LocalDensity.current
    val thumbPx = with(density) { THUMB_HEIGHT.toPx() }
    val scrolling = listState.isScrollInProgress
    val visible = (scrolling || dragging) && items.size > MIN_ITEMS_FOR_SCROLLER
    val alpha by animateFloatAsState(if (visible) 1f else 0f, if (level == MotionLevel.FULL || level == MotionLevel.REDUCED) MaterialTheme.motionScheme.defaultEffectsSpec() else snap(), label = "scroller")
    if (alpha <= 0f && !visible) return
    val total = items.size
    val shownFraction = if (dragging) dragFraction else {
        val maxIndex = (total - listState.layoutInfo.visibleItemsInfo.size).coerceAtLeast(1)
        (listState.firstVisibleItemIndex.toFloat() / maxIndex).coerceIn(0f, 1f)
    }
    val travel = (trackPx - thumbPx).coerceAtLeast(0f)
    val labelAt: (Int) -> String? = { index ->
        items.subList(0, (index + 1).coerceIn(0, total)).lastOrNull { it is ListItem.Header }?.let { labelOf((it as ListItem.Header).bucket) }
    }
    Box(modifier.fillMaxHeight().width(48.dp).onSizeChanged { trackPx = it.height }.graphicsLayer { this.alpha = alpha }.testTag(InboxTags.FAST_SCROLLER).clearAndSetSemantics { }) {
        Box(
            Modifier
                .offset { IntOffset(0, (shownFraction * travel).roundToInt()) }
                .width(48.dp).height(THUMB_HEIGHT)
                .draggable(
                    orientation = Orientation.Vertical,
                    state = rememberDraggableState { dy ->
                        if (travel > 0f) {
                            dragFraction = (dragFraction + dy / travel).coerceIn(0f, 1f)
                            val index = (dragFraction * (total - 1)).roundToInt().coerceIn(0, total - 1)
                            scope.launch { listState.scrollToItem(index) }
                            val label = labelAt(index)
                            if (label != null && label != lastLabel) { lastLabel = label; haptics.perform(HapticEvent.Tick) }
                        }
                    },
                    onDragStarted = {
                        dragFraction = (listState.firstVisibleItemIndex.toFloat() / (total - 1).coerceAtLeast(1)).coerceIn(0f, 1f)
                        lastLabel = labelAt(listState.firstVisibleItemIndex)
                        dragging = true
                    },
                    onDragStopped = { dragging = false },
                ),
            contentAlignment = Alignment.CenterEnd,
        ) {
            Box(Modifier.padding(end = 4.dp).width(8.dp).fillMaxHeight().clip(MaterialTheme.shapes.small).background(MaterialTheme.colorScheme.outline))
            if (dragging && lastLabel != null) {
                Surface(
                    Modifier.offset(x = (-56).dp).widthIn(min = 48.dp),
                    shape = MaterialTheme.shapes.large,
                    color = MaterialTheme.colorScheme.inverseSurface,
                    contentColor = MaterialTheme.colorScheme.inverseOnSurface,
                ) { Text(lastLabel!!, Modifier.padding(horizontal = 12.dp, vertical = 8.dp), style = MaterialTheme.typography.labelLarge, maxLines = 1) }
            }
        }
    }
}

private val THUMB_HEIGHT = 64.dp
private const val MIN_ITEMS_FOR_SCROLLER = 30
