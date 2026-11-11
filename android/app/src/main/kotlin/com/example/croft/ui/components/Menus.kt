// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription

/** One menu entry. */
class MenuItemSpec(
    val label: String,
    val onClick: () -> Unit,
    val icon: ImageVector? = null,
    /** Last in the menu, in the error colour (Remove, Delete). */
    val destructive: Boolean = false,
    /** An item that cannot apply is hidden. */
    val visible: Boolean = true,
    /** An item shown but not usable, with the reason (shown under the label). Only when the reason helps. */
    val disabledReason: String? = null,
)

/** The rules of a menu (docs/ux/controls.md): at most seven items, destructive ones last, no submenus. */
class MenuModel private constructor(val items: List<MenuItemSpec>) {
    companion object {
        /** More than this opens a sheet instead. */
        const val MAX_ITEMS = 7

        /** The menu for [all]: hidden items dropped, then the rules checked. Throws for a menu that breaks them (a bug). */
        fun of(all: List<MenuItemSpec>): MenuModel {
            val items = all.filter { it.visible }
            require(items.size <= MAX_ITEMS) { "A menu has at most $MAX_ITEMS items; use a sheet (got ${items.size})" }
            val firstDestructive = items.indexOfFirst { it.destructive }
            require(firstDestructive == -1 || items.drop(firstDestructive).all { it.destructive }) { "Destructive items come last" }
            return MenuModel(items)
        }
    }
}

/**
 * A menu: seven items at most, destructive items last in the error colour, a disabled item with its reason, mirrored in
 * right-to-left by the platform, closed by Back or a tap outside. Arrow keys, Enter and Escape work with a hardware
 * keyboard (the items are focusable). [header] is the title line (the real address of a link, the mailbox name).
 */
@Composable
fun CroftMenu(
    expanded: Boolean,
    onDismiss: () -> Unit,
    items: List<MenuItemSpec>,
    modifier: Modifier = Modifier,
    header: (@Composable () -> Unit)? = null,
) {
    val model = MenuModel.of(items)
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss, modifier = modifier) {
        if (header != null) { header(); HorizontalDivider() }
        model.items.forEach { item ->
            val enabled = item.disabledReason == null
            DropdownMenuItem(
                text = {
                    Column {
                        Text(item.label, color = if (item.destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)
                        if (item.disabledReason != null) Text(item.disabledReason, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                },
                leadingIcon = item.icon?.let { { Icon(it, contentDescription = null, tint = if (item.destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant) } },
                enabled = enabled,
                onClick = { onDismiss(); item.onClick() },
                modifier = if (item.disabledReason != null) Modifier.semantics { stateDescription = item.disabledReason } else Modifier,
            )
        }
    }
}
