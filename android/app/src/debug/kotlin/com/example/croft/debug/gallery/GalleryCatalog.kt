// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug.gallery

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Create
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import com.example.croft.ui.components.AccountStatus
import com.example.croft.ui.components.AccountStatusChip
import com.example.croft.ui.components.AttachmentChip
import com.example.croft.ui.components.AttachmentState
import com.example.croft.ui.components.BannerHost
import com.example.croft.ui.components.BannerKind
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.ButtonProgressController
import com.example.croft.ui.components.CroftBottomSheet
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftCheckboxRow
import com.example.croft.ui.components.CroftExtendedFab
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.CroftRadioRow
import com.example.croft.ui.components.CroftSuggestionChip
import com.example.croft.ui.components.CroftSwitchRow
import com.example.croft.ui.components.CroftTextField
import com.example.croft.ui.components.DestructiveDialog
import com.example.croft.ui.components.EncryptionMark
import com.example.croft.ui.components.FabState
import com.example.croft.ui.components.FieldValidation
import com.example.croft.ui.components.ForcedState
import com.example.croft.ui.components.MenuItemSpec
import com.example.croft.ui.components.MessageRow
import com.example.croft.swipe.SwipeAction
import com.example.croft.ui.components.SwipeCardLook
import com.example.croft.ui.swipe.SwipeRowFrozen
import com.example.croft.ui.components.MessageRowState
import com.example.croft.ui.components.RecipientChip
import com.example.croft.ui.components.RecipientChipState
import com.example.croft.ui.components.SendActions
import com.example.croft.ui.components.SendButton
import com.example.croft.ui.components.SendCondition
import com.example.croft.ui.components.CroftIcons
import com.example.croft.ui.components.longPressLift
import com.example.croft.ui.components.rememberForcedInteractionSource
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LocalHaptics
import kotlinx.coroutines.launch

private fun id(section: String, name: String) = "$section.$name"
private val NoOp: () -> Unit = {}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Wrap(content: @Composable () -> Unit) {
    FlowRow(Modifier.fillMaxWidth().padding(4.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) { content() }
}

@Composable
private fun Caption(text: String) = Text(text, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)

/** One labelled item of an entry that shows several states side by side. */
@Composable
private fun Item(caption: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) { Caption(caption); content() }
}

private val sampleRow = MessageRowState()

/** The whole catalogue: every control of docs/ux/controls.md in every state it has. */
object GalleryCatalog {
    val sections: List<GallerySection> by lazy {
        listOf(common(), buttons(), fab(), send(), chips(), fields(), toggles(), rows(), swipes(), listMarkers(), attachments(), banners(), snackbars(), dialogs(), sheets(), menus(), iconButtons(), longPress(), alphabetIndex(), calendarParts())
    }

    // ---- Calendar day header and event card (S34, S36, T39b) ----
    private fun calendarParts(): GallerySection {
        val zone = java.time.ZoneOffset.UTC
        fun occ(title: String, allDay: Boolean, startHour: Int, location: String? = null, zoneUnknown: Boolean = false, unsupported: Boolean = false) =
            com.example.croft.mail.calendar.Occurrence(
                eventId = 1, account = 1, calendar = 1, color = null,
                startMs = 1_794_787_200_000L + startHour * 3_600_000L, endMs = 1_794_787_200_000L + (startHour + 1) * 3_600_000L,
                allDay = allDay, keyMs = 0, title = title, location = location, repeats = false,
                repeatsUnsupported = unsupported, zoneUnknown = zoneUnknown, pending = false,
            )
        fun card(name: String, label: String, o: com.example.croft.mail.calendar.Occurrence, overlaps: Boolean = false) =
            GalleryEntry(id("calendar", name), label, emptyList()) {
                var tapped by remember { mutableStateOf("Tap the card") }
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    com.example.croft.ui.calendar.EventCard(o, zone, overlaps, onClick = { tapped = "Opened" })
                    Caption(tapped)
                }
            }
        return GallerySection(
            "calendar", "Calendar day header and event card",
            listOf(
                GalleryEntry(id("calendar", "day"), "Day header (a day, and today in a filled circle)", emptyList()) {
                    Wrap {
                        com.example.croft.ui.calendar.DayHeader(java.time.LocalDate.of(2026, 11, 7), today = false)
                        com.example.croft.ui.calendar.DayHeader(java.time.LocalDate.of(2026, 11, 6), today = true)
                    }
                },
                card("timed", "Event card: title, times and place", occ("Dentist", false, 15, "Mill Road Surgery")),
                card("all_day", "Event card: all day", occ("Bank holiday", true, 0)),
                card("overlap", "Event card: overlaps another event (warning icon)", occ("Standup", false, 12), overlaps = true),
                card("notes", "Event card: time zone unknown, repeats Croft can't show", occ("Imported", false, 9, zoneUnknown = true, unsupported = true)),
            ),
        )
    }

    // ---- A to Z index (S40, T38b) ----
    private fun alphabetIndex(): GallerySection {
        val present = setOf("A", "B", "H", "M", "N", "P", "#")
        fun e(name: String, label: String, bubble: String?) = GalleryEntry(id("index", name), label, emptyList()) {
            var picked by remember { mutableStateOf("Drag over the letters") }
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                androidx.compose.foundation.layout.Box(Modifier.fillMaxWidth().height(432.dp)) {
                    com.example.croft.ui.contacts.AlphabetIndex(
                        letters = com.example.croft.mail.contacts.ContactSections.INDEX,
                        present = present,
                        onLetter = { picked = "Jumped to $it" },
                        modifier = Modifier.align(Alignment.CenterEnd),
                        forceBubble = bubble,
                    )
                }
                Caption(picked)
            }
        }
        return GallerySection(
            "index", "A to Z index",
            listOf(
                e("idle", "Idle (letters without a section are faded)", null),
                e("dragging", "Dragging (the letter bubble beside the finger, a Tick at each new letter)", "M"),
            ),
        )
    }

    fun entry(id: String): GalleryEntry? = sections.firstNotNullOfOrNull { s -> s.entries.firstOrNull { it.id == id } }

    // ---- States common to all controls ----
    private fun common(): GallerySection {
        fun one(name: String, label: String, row: String, content: @Composable () -> Unit) =
            GalleryEntry(id("common", name), label, listOf(row), { content() })
        return GallerySection(
            "common", "States common to all controls",
            listOf(
                one("enabled", "Enabled", "Enabled") { CroftButton("Archive", NoOp, kind = ButtonKind.TONAL) },
                one("hovered", "Hovered (web, mouse)", "Hovered (web, mouse)") { CroftButton("Archive", NoOp, kind = ButtonKind.TONAL, interactionSource = rememberForcedInteractionSource(ForcedState.HOVERED)) },
                one("focused", "Focused (keyboard, switch access)", "Focused (keyboard, switch access)") { CroftButton("Archive", NoOp, kind = ButtonKind.TONAL, interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED)) },
                one("pressed", "Pressed", "Pressed") { CroftButton("Archive", NoOp, kind = ButtonKind.TONAL, interactionSource = rememberForcedInteractionSource(ForcedState.PRESSED)) },
                one("long_pressed", "Long-pressed (1 dp lift, 102 percent)", "Long-pressed") {
                    var shown by remember { mutableStateOf("Hold the card") }
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        MessageRow("Mira Okafor", "Re: Server migration notes", "Thanks, I will test the new", "Mon", MessageRowState(unread = true), onClick = NoOp, onLongClick = { shown = "Selection opened" }, forceLifted = true)
                        Caption(shown)
                    }
                },
                one("disabled", "Disabled (38 percent content, 12 percent container)", "Disabled") { CroftButton("Archive", NoOp, kind = ButtonKind.TONAL, enabled = false) },
                one("loading", "Loading (Busy, width unchanged)", "Loading") { CroftButton("Archive", NoOp, kind = ButtonKind.TONAL, progress = ButtonProgress.LOADING) },
                one("error", "Error (outline, icon and message)", "Error") { CroftButton("Archive", NoOp, kind = ButtonKind.OUTLINED, errorText = "Could not reach the server") },
            ),
        )
    }

    // ---- Buttons ----
    private fun buttons(): GallerySection {
        fun kind(k: ButtonKind, name: String, row: String) = GalleryEntry(id("buttons", name), name.replaceFirstChar { it.uppercase() }, listOf(row)) {
            Wrap {
                Item("Enabled") { CroftButton("Label", NoOp, kind = k) }
                Item("Disabled") { CroftButton("Label", NoOp, kind = k, enabled = false) }
                Item("Loading") { CroftButton("Label", NoOp, kind = k, progress = ButtonProgress.LOADING) }
                Item("Success") { CroftButton("Label", NoOp, kind = k, progress = ButtonProgress.SUCCESS) }
                if (k == ButtonKind.OUTLINED || k == ButtonKind.TEXT) Item("Error") { CroftButton("Label", NoOp, kind = k, errorText = "Something went wrong") }
            }
        }
        return GallerySection(
            "buttons", "Buttons",
            listOf(
                kind(ButtonKind.FILLED, "filled", "Filled"),
                kind(ButtonKind.TONAL, "tonal", "Tonal"),
                kind(ButtonKind.OUTLINED, "outlined", "Outlined"),
                kind(ButtonKind.TEXT, "text", "Text"),
                GalleryEntry(id("buttons", "danger"), "Danger (filled and text)", listOf("Danger (filled or text)")) {
                    Wrap {
                        Item("Filled") { CroftButton("Delete forever", NoOp, kind = ButtonKind.DANGER) }
                        Item("Text") { CroftButton("Remove account", NoOp, kind = ButtonKind.DANGER_TEXT) }
                        Item("Disabled") { CroftButton("Discard", NoOp, kind = ButtonKind.DANGER, enabled = false) }
                        Item("Needs a confirmation") { Text("Always asked first (see Dialog)", style = MaterialTheme.typography.bodySmall) }
                    }
                },
                GalleryEntry(id("buttons", "run_demo"), "Loading, success, back (try it)", emptyList()) {
                    val controller = remember { ButtonProgressController() }
                    val scope = rememberCoroutineScope()
                    CroftButton("Save", { scope.launch { controller.run { kotlinx.coroutines.delay(DEMO_WORK_MS); true } } }, progress = controller.progress)
                },
            ),
        )
    }

    // ---- Extended FAB ----
    private fun fab(): GallerySection = GallerySection(
        "fab", "Extended FAB",
        listOf(
            GalleryEntry(id("fab", "expanded"), "Expanded", listOf("Extended FAB")) { Wrap { CroftExtendedFab("Compose", NoOp, FabState.EXPANDED) } },
            GalleryEntry(id("fab", "collapsed"), "Collapsed (icon only, on scroll down)", listOf("Extended FAB")) { Wrap { CroftExtendedFab("Compose", NoOp, FabState.COLLAPSED) } },
            GalleryEntry(id("fab", "hidden"), "Hidden (selection mode, keyboard open): nothing is drawn", listOf("Extended FAB")) {
                Wrap { CroftExtendedFab("Compose", NoOp, FabState.HIDDEN); Text("(hidden)", style = MaterialTheme.typography.bodySmall) }
            },
            GalleryEntry(id("fab", "pressed"), "Pressed", listOf("Extended FAB")) { Wrap { CroftExtendedFab("Compose", NoOp, FabState.EXPANDED, interactionSource = rememberForcedInteractionSource(ForcedState.PRESSED)) } },
            GalleryEntry(id("fab", "focused"), "Focused", listOf("Extended FAB")) { Wrap { CroftExtendedFab("Compose", NoOp, FabState.EXPANDED, interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED)) } },
        ),
    )

    // ---- Send ----
    private fun send(): GallerySection {
        val rows = mapOf(
            SendCondition.NO_RECIPIENT to "No recipient",
            SendCondition.INVALID_RECIPIENT to "A recipient chip has an invalid address",
            SendCondition.EMPTY_CONFIRM to "Recipient but empty subject and body",
            SendCondition.READY to "Everything present",
            SendCondition.OVER_SIZE_LIMIT to "Attachments over the server's size limit",
            SendCondition.UPLOADING to "Attachment still uploading",
            SendCondition.OFFLINE_OR_SIGN_IN to "Offline, or the account needs sign-in",
            SendCondition.SENDING to "Tapped",
            SendCondition.SCHEDULED to "Scheduled",
            SendCondition.ENCRYPTION_NO_KEY to "Encryption required but a recipient has no key",
        )
        return GallerySection(
            "send", "Send button",
            SendCondition.entries.map { c ->
                GalleryEntry(id("send", c.name.lowercase()), c.name.lowercase().replace('_', ' '), listOf(rows.getValue(c)) + (if (c == SendCondition.READY) listOf("Send") else emptyList())) {
                    val haptics = LocalHaptics.current
                    var result by remember { mutableStateOf("Tap or long-press") }
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SendButton(
                            c,
                            SendActions(
                                onSend = { haptics.perform(HapticEvent.Success); result = "Sent: Undo snackbar" },
                                onAskEmpty = { result = "Asks: Send without subject or text?" },
                                onEncryptionCheck = { result = "Opens S24, the encryption check" },
                                onSchedule = { result = "Opens Schedule send (S25)" },
                                onExplain = { reason -> result = "Snackbar: $reason" },
                            ),
                        )
                        Caption(result)
                    }
                }
            },
        )
    }

    // ---- Chips ----
    private fun chips(): GallerySection = GallerySection(
        "chips", "Chips",
        listOf(
            GalleryEntry(id("chips", "filter"), "Filter chip", listOf("Filter chip (category, search filters)")) {
                var selected by remember { mutableStateOf(true) }
                Wrap {
                    Item("Unselected") { CroftFilterChip("Unread", false, NoOp) }
                    Item("Selected (tap me)") { CroftFilterChip("Unread", selected, { selected = !selected }) }
                    Item("Pressed") { CroftFilterChip("Unread", false, NoOp, interactionSource = rememberForcedInteractionSource(ForcedState.PRESSED)) }
                    Item("Focused") { CroftFilterChip("Unread", false, NoOp, interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED)) }
                    Item("Disabled") { CroftFilterChip("Unread", false, NoOp, enabled = false) }
                    Item("With count") { CroftFilterChip("Updates", true, NoOp, count = 12) }
                }
            },
            GalleryEntry(id("chips", "input"), "Input chip (recipient)", listOf("Input chip (recipients)")) {
                var menu by remember { mutableStateOf(false) }
                Wrap {
                    Item("Normal (tap: menu)") {
                        Column {
                            RecipientChip("Mira Okafor", RecipientChipState.NORMAL, { menu = true }, onLongPress = NoOp)
                            CroftMenu(menu, { menu = false }, listOf(MenuItemSpec("Move to Cc", NoOp), MenuItemSpec("Move to Bcc", NoOp), MenuItemSpec("View key", NoOp, icon = Icons.Filled.Lock), MenuItemSpec("Remove", NoOp, destructive = true)))
                        }
                    }
                    Item("Selected") { RecipientChip("Mira Okafor", RecipientChipState.SELECTED, NoOp) }
                    Item("Error") { RecipientChip("mira@@example", RecipientChipState.ERROR, NoOp) }
                    Item("Key found") { RecipientChip("Sam Rivera", RecipientChipState.NORMAL, NoOp, hasKey = true) }
                    Item("Lifted (long-press)") { RecipientChip("Dan Hollis", RecipientChipState.NORMAL, NoOp, onLongPress = NoOp, forceLifted = true) }
                }
            },
            GalleryEntry(id("chips", "suggestion"), "Suggestion chip", listOf("Suggestion chip")) {
                Wrap {
                    Item("Normal") { CroftSuggestionChip("Thanks!", NoOp) }
                    Item("Pressed") { CroftSuggestionChip("Thanks!", NoOp, interactionSource = rememberForcedInteractionSource(ForcedState.PRESSED)) }
                }
            },
            GalleryEntry(id("chips", "account_status"), "Account status chip", listOf("Account status chip")) {
                Wrap {
                    AccountStatus.entries.forEach { s ->
                        Item(s.name.lowercase().replace('_', ' ') + if (s == AccountStatus.CONNECTED) " (hidden)" else "") { AccountStatusChip("Sam", s, NoOp) }
                    }
                }
            },
        ),
    )

    // ---- Text fields ----
    private fun fields(): GallerySection {
        fun f(name: String, label: String, content: @Composable () -> Unit) = GalleryEntry(id("fields", name), label, emptyList()) { content() }
        return GallerySection(
            "fields", "Text fields",
            listOf(
                f("empty", "Empty with label") { CroftTextField("", {}, "Email address") },
                f("focused", "Focused (label floats, 2 dp outline)") {
                    CroftTextField("", {}, "Email address", interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED))
                },
                f("filled", "Filled") { CroftTextField("sam@example.org", {}, "Email address") },
                f("error", "Error (icon and message)") { CroftTextField("sam@", {}, "Email address", errorText = "Enter a full address like sam@example.org") },
                f("disabled", "Disabled") { CroftTextField("sam@example.org", {}, "Email address", enabled = false) },
                f("read_only", "Read-only") { CroftTextField("sam@example.org", {}, "Email address", readOnly = true) },
                f("icons", "Leading and trailing icons") { CroftTextField("", {}, "Search mail", leadingIcon = Icons.Filled.Search, trailingIcon = Icons.Filled.Settings) },
                f("supporting_counter", "Supporting text and counter") { CroftTextField("Lunch", {}, "Subject", supportingText = "Shown in the list", maxLength = 78) },
                f("password", "Password (Show and Hide)") { CroftTextField("hunter2 (made up)", {}, "Password", password = true) },
                f("validation", "Validated on leaving the field (try it)") {
                    val v = remember { FieldValidation { if ("@" in it) null else "Enter a full address like sam@example.org" } }
                    CroftTextField(v.value, v::onValueChange, "Email address", errorText = v.error, onFocusChange = v::onFocusChange)
                },
            ),
        )
    }

    // ---- Switches, radios, checkboxes ----
    private fun toggles(): GallerySection {
        fun t(name: String, label: String, row: List<String>, content: @Composable () -> Unit) = GalleryEntry(id("toggles", name), label, row) { content() }
        return GallerySection(
            "toggles", "Switches, radios, checkboxes",
            listOf(
                t("switch_off", "Switch off", emptyList()) { CroftSwitchRow("Sync over mobile data", false, {}) },
                t("switch_on", "Switch on (try it: haptic)", emptyList()) {
                    var on by remember { mutableStateOf(true) }
                    CroftSwitchRow("Sync over mobile data", on, { on = it }, hint = "Takes effect after Croft restarts")
                },
                t("switch_pressed", "Switch pressed (thumb grows)", emptyList()) { CroftSwitchRow("Sync over mobile data", false, {}, interactionSource = rememberForcedInteractionSource(ForcedState.PRESSED)) },
                t("switch_disabled_on", "Switch disabled, on", emptyList()) { CroftSwitchRow("Sync over mobile data", true, {}, enabled = false) },
                t("switch_disabled_off", "Switch disabled, off", emptyList()) { CroftSwitchRow("Sync over mobile data", false, {}, enabled = false) },
                t("radio", "Radio rows (whole row taps)", emptyList()) {
                    var sel by remember { mutableIntStateOf(1) }
                    Column { listOf("Never", "Wi-Fi only", "Always").forEachIndexed { i, s -> CroftRadioRow(s, sel == i, { sel = i }) }; CroftRadioRow("Disabled option", false, {}, enabled = false) }
                },
                t("checkbox", "Checkbox: off, on, indeterminate", emptyList()) {
                    var a by remember { mutableStateOf(true) }
                    var b by remember { mutableStateOf(false) }
                    val all = when { a && b -> ToggleableState.On; !a && !b -> ToggleableState.Off; else -> ToggleableState.Indeterminate }
                    Column {
                        CroftCheckboxRow("Select all", all, { a = it; b = it })
                        CroftCheckboxRow("Inbox", if (a) ToggleableState.On else ToggleableState.Off, { a = it })
                        CroftCheckboxRow("Sent", if (b) ToggleableState.On else ToggleableState.Off, { b = it })
                        CroftCheckboxRow("Disabled", ToggleableState.Off, {}, enabled = false)
                    }
                },
            ),
        )
    }

    // ---- Message row ----
    private fun rows(): GallerySection {
        fun r(name: String, row: String, state: MessageRowState, source: ForcedState = ForcedState.NONE, drag: Boolean = false, hold: Long? = null, lifted: Boolean = false) =
            GalleryEntry(id("row", name), row, listOf(row)) {
                val interaction = rememberForcedInteractionSource(source)
                val content: @Composable (SwipeCardLook?) -> Unit = { card ->
                    MessageRow(
                        "Sam Rivera", "Lunch on Friday?", "Are you free around noon? I booked the place by the harbour.", "9:12", state, NoOp,
                        onLongClick = NoOp, onAvatarClick = NoOp, onToggleStar = NoOp,
                        card = card, failedEdgeHoldMs = hold, forceLifted = lifted,
                        interactionSource = interaction,
                    )
                }
                if (drag) SwipeRowFrozen(offsetDp = -120f, action = SwipeAction.DELETE, armed = false) { card -> content(card) } else content(null)
            }
        return GallerySection(
            "row", "Message row",
            listOf(
                r("read", "Read", MessageRowState()),
                r("unread", "Unread", MessageRowState(unread = true)),
                r("pressed", "Pressed", MessageRowState(), source = ForcedState.PRESSED),
                r("focused", "Focused (keyboard)", MessageRowState(), source = ForcedState.FOCUSED),
                r("selected", "Selected", MessageRowState(selected = true)),
                r("dragging", "Dragging (swipe)", MessageRowState(unread = true), drag = true),
                r("new", "New (just arrived)", MessageRowState(unread = true, isNew = true)),
                r("starred", "Starred", MessageRowState(starred = true)),
                r("encrypted", "Encrypted", MessageRowState(encryption = EncryptionMark.ENCRYPTED)),
                r("signed_invalid", "Encrypted", MessageRowState(encryption = EncryptionMark.SIGNED_INVALID)),
                r("pending", "Action pending offline", MessageRowState(pendingOffline = true)),
                r("failed", "Action failed", MessageRowState(failed = true), hold = null),
                r("lifted", "Long-pressed", MessageRowState(), lifted = true),
            ),
        )
    }

    // ---- Swipe (T15): the card and the pill in every state ----
    private fun swipes(): GallerySection {
        fun s(name: String, label: String, action: SwipeAction, offsetDp: Float, armed: Boolean, available: Boolean = true, state: MessageRowState = MessageRowState(unread = true)) =
            GalleryEntry(id("swipe", name), label, emptyList()) {
                SwipeRowFrozen(offsetDp = offsetDp, action = action, armed = armed, available = available) { card ->
                    MessageRow(
                        "Sam Rivera", "Lunch on Friday?", "Are you free around noon? I booked the place by the harbour.", "9:12", state, NoOp,
                        card = card, timeDescription = "2 hours ago",
                    )
                }
            }
        return GallerySection(
            "swipe", "Swipe: the card and the pill",
            listOf(
                s("read_dragging", "Mark read, below the arm point (tonal pill)", SwipeAction.TOGGLE_READ, 110f, armed = false),
                s("read_armed", "Mark read, armed (full colour, icon grown)", SwipeAction.TOGGLE_READ, 170f, armed = true),
                s("delete_dragging", "Delete, below the arm point", SwipeAction.DELETE, -110f, armed = false),
                s("delete_armed", "Delete, armed", SwipeAction.DELETE, -190f, armed = true),
                s("delete_committing", "Delete, committing (pill nearly row wide, card leaving)", SwipeAction.DELETE, -270f, armed = true),
                s("archive_armed", "Archive, armed", SwipeAction.ARCHIVE, 170f, armed = true),
                s("spam_armed", "Mark as spam, armed", SwipeAction.SPAM, -190f, armed = true),
                s("star_armed", "Star, armed", SwipeAction.TOGGLE_STAR, 170f, armed = true),
                s("snooze_parked", "Snooze, parked at 40 percent", SwipeAction.SNOOZE, 140f, armed = true),
                s("move_parked", "Move to, parked at 40 percent", SwipeAction.MOVE_TO, 140f, armed = true),
                s("unavailable", "Not available here (grey, never arms)", SwipeAction.ARCHIVE, -190f, armed = false, available = false),
                s("narrow", "Just started: a round capsule, no icon yet", SwipeAction.DELETE, -30f, armed = false),
            ),
        )
    }

    // ---- Message row markers used by the message list (T14) ----
    private fun listMarkers(): GallerySection {
        fun r(name: String, label: String, state: MessageRowState, sender: String = "Sam Rivera", outgoing: Boolean = false, swipe: Boolean = true) =
            GalleryEntry(id("listmarkers", name), label, emptyList()) {
                MessageRow(
                    sender, "Lunch on Friday?", "Are you free around noon? I booked the place by the harbour.", "9:12", state, NoOp,
                    onLongClick = NoOp, onAvatarClick = NoOp, onToggleStar = NoOp, outgoing = outgoing, swipeEnabled = swipe,
                    timeDescription = "2 hours ago",
                )
            }
        return GallerySection(
            "listmarkers", "Message row markers (message list)",
            listOf(
                r("attachment", "Attachment clip", MessageRowState(hasAttachment = true)),
                r("thread", "Conversation count chip", MessageRowState(threadCount = 4, unread = true)),
                r("everything", "Unread, star, clip, lock, count and clock together", MessageRowState(unread = true, starred = true, hasAttachment = true, encryption = EncryptionMark.ENCRYPTED, threadCount = 12, pendingOffline = true)),
                r("outgoing", "Sent: recipients instead of the sender", MessageRowState(), sender = "Mira Okafor", outgoing = true),
                r("swipe_off", "Selection mode: swipe off", MessageRowState(selected = true), swipe = false),
            ),
        )
    }

    // ---- Attachment chip ----
    private fun attachments(): GallerySection = GallerySection(
        "attachment", "Attachment chip",
        AttachmentState.entries.map { s ->
            GalleryEntry(id("attachment", s.name.lowercase()), s.name.lowercase().replace('_', ' '), listOf("Attachment chip")) {
                AttachmentChip(
                    "Quarterly report.pdf", "2.4 MB", s, NoOp, progress = 0.4f, onLongClick = NoOp,
                    limitText = if (s == AttachmentState.OVER_LIMIT) "Over the 25 MB limit" else null,
                )
            }
        },
    )

    // ---- Banners ----
    private fun banners(): GallerySection = GallerySection(
        "banner", "Banner",
        BannerKind.entries.map { k ->
            GalleryEntry(id("banner", k.name.lowercase()), k.name.lowercase().replace('_', ' '), listOf("Banner")) {
                BannerHost(listOf(k), onAction = {}, onDismiss = {}, accountName = "Sam")
            }
        } + GalleryEntry(id("banner", "most_urgent"), "All active: only the most urgent shows", listOf("Banner")) {
            BannerHost(BannerKind.entries, onAction = {}, onDismiss = {}, accountName = "Sam")
        },
    )

    // ---- Snackbar ----
    private fun snackbars(): GallerySection = GallerySection(
        "snackbar", "Snackbar",
        listOf(
            GalleryEntry(id("snackbar", "replacement"), "One at a time: a new one replaces the old", listOf("Snackbar")) {
                var n by remember { mutableIntStateOf(0) }
                Wrap {
                    CroftButton("Delete message", { n++; snackbar.show("Message $n deleted", "Undo", CroftSnackbarStateWindow) {} }, kind = ButtonKind.TONAL)
                    CroftButton("Archive message", { n++; snackbar.show("Message $n archived", "Undo", CroftSnackbarStateWindow) {} }, kind = ButtonKind.TONAL)
                    CroftButton("Say something", { snackbar.show("Saved") }, kind = ButtonKind.TEXT)
                }
            },
        ),
    )

    // ---- Dialog ----
    private fun dialogs(): GallerySection = GallerySection(
        "dialog", "Dialog (destructive or irreversible only)",
        listOf(
            GalleryEntry(id("dialog", "destructive"), "Destructive dialog", listOf("Danger (filled or text)")) {
                var open by remember { mutableStateOf(false) }
                var result by remember { mutableStateOf("Not opened") }
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    CroftButton("Delete forever", { open = true }, kind = ButtonKind.DANGER_TEXT)
                    Caption(result)
                }
                if (open) DestructiveDialog(
                    title = "Delete forever?", message = "This message cannot be brought back.", confirmLabel = "Delete forever",
                    onConfirm = { open = false; result = "Confirmed (haptic played)" }, onDismiss = { open = false; result = "Cancelled" },
                )
            },
        ),
    )

    // ---- Bottom sheet ----
    private fun sheets(): GallerySection = GallerySection(
        "sheet", "Bottom sheet",
        listOf(
            GalleryEntry(id("sheet", "with_input"), "Sheet that holds unsaved input", emptyList()) {
                var open by remember { mutableStateOf(false) }
                CroftButton("Open sheet", { open = true }, kind = ButtonKind.TONAL)
                if (open) {
                    var text by remember { mutableStateOf("") }
                    CroftBottomSheet(onDismiss = { open = false }, hasUnsavedInput = text.isNotEmpty()) {
                        Text("Snooze until", style = MaterialTheme.typography.titleMedium)
                        CroftTextField(text, { text = it }, "Custom time")
                    }
                }
            },
        ),
    )

    // ---- Menus ----
    private fun menus(): GallerySection = GallerySection(
        "menu", "Menus (at most seven items, destructive last)",
        listOf(
            GalleryEntry(id("menu", "conversation"), "Overflow menu", listOf("Icon-only button")) {
                var open by remember { mutableStateOf(false) }
                var picked by remember { mutableStateOf("") }
                Column {
                    CroftIconButton(Icons.Filled.Settings, "More options", { open = true })
                    CroftMenu(
                        open, { open = false },
                        listOf(
                            MenuItemSpec("Mark as unread", { picked = "Mark as unread" }, icon = Icons.Filled.Email),
                            MenuItemSpec("Move to", { picked = "Move to" }),
                            MenuItemSpec("Snooze", { picked = "Snooze" }),
                            MenuItemSpec("Print", NoOp, disabledReason = "No printer is set up"),
                            MenuItemSpec("Share", { picked = "Share" }, icon = Icons.Filled.Share),
                            MenuItemSpec("Block sender", { picked = "Block sender" }, destructive = true),
                            MenuItemSpec("Delete", { picked = "Delete" }, icon = Icons.Filled.Delete, destructive = true),
                        ),
                    )
                    if (picked.isNotEmpty()) Caption("Picked: $picked")
                }
            },
        ),
    )

    // ---- Icon-only button ----
    private fun iconButtons(): GallerySection = GallerySection(
        "iconbutton", "Icon-only button with tooltip",
        listOf(
            GalleryEntry(id("iconbutton", "states"), "Enabled, pressed, focused, disabled", listOf("Icon-only button")) {
                Wrap {
                    Item("Enabled") { CroftIconButton(Icons.Filled.Delete, "Delete", NoOp) }
                    Item("Pressed") { CroftIconButton(Icons.Filled.Delete, "Delete", NoOp, interactionSource = rememberForcedInteractionSource(ForcedState.PRESSED)) }
                    Item("Focused") { CroftIconButton(Icons.Filled.Delete, "Delete", NoOp, interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED)) }
                    Item("Disabled") { CroftIconButton(Icons.Filled.Add, "Add", NoOp, enabled = false, disabledReason = "Nothing to add yet") }
                    Item("Compose") { CroftIconButton(Icons.Filled.Create, "Compose", NoOp) }
                    Item("Schedule") { CroftIconButton(CroftIcons.Schedule, "Schedule send", NoOp) }
                }
            },
        ),
    )

    // ---- Long-press ----
    private fun longPress(): GallerySection = GallerySection(
        "longpress", "Long-press",
        listOf(
            GalleryEntry(id("longpress", "demo"), "Hold the card: lift, haptic, then the result", listOf("Long-pressed")) {
                var result by remember { mutableStateOf("Hold for the system long-press time") }
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    androidx.compose.material3.Surface(
                        modifier = Modifier.longPressLift { result = "Result opened at the moment of the long-press" },
                        shape = MaterialTheme.shapes.large,
                        color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ) { Text("Hold me", Modifier.padding(24.dp)) }
                    Caption(result)
                }
            },
        ),
    )
}

/** The Undo window of the default setting, so the demo snackbars last as long as the real ones. */
private val CroftSnackbarStateWindow: Long = com.example.croft.ui.components.CroftSnackbarState.DefaultUndoWindowMs
private const val DEMO_WORK_MS = 1_500L
