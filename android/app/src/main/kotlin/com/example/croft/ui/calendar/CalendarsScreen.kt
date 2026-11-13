// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.calendar.CalendarController
import com.example.croft.ui.components.CroftIconButton

/** Test tags of S39. */
object CalendarsTags {
    const val SCREEN = "calendars-screen"
    fun calendar(id: Long) = "calendars-calendar-$id"
}

/** S39 Calendars: grouped by account, each with its colour and a check for whether its events show. A hidden one stays in sync. */
@Composable
fun CalendarsScreen(controller: CalendarController, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    BackHandler { onBack() }
    Column(
        modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(CalendarsTags.SCREEN)
            .windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars),
    ) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            CroftIconButton(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.event_back), onBack)
            Text(stringResource(R.string.calendars_title), Modifier.padding(start = 8.dp).semantics { heading() }, style = MaterialTheme.typography.titleLarge)
        }
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (a in state.accounts) {
                Text(a.label, Modifier.padding(top = 12.dp, bottom = 4.dp).semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                for (c in state.calendars[a.id].orEmpty()) {
                    val color = calendarColor(c.color, MaterialTheme.colorScheme.primary)
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(MaterialTheme.shapes.medium)
                            .toggleable(value = c.shown, role = Role.Checkbox, onValueChange = { controller.setShown(a.id, c.id, it) })
                            .padding(horizontal = 8.dp)
                            .testTag(CalendarsTags.calendar(c.id)),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        Checkbox(checked = c.shown, onCheckedChange = null)
                        Text(c.name, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                        Box(Modifier.size(16.dp).clip(CircleShape).background(color).clearAndSetSemantics { })
                    }
                }
            }
            Text(stringResource(R.string.calendars_note), Modifier.padding(top = 16.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
