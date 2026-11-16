// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.calendar.RsvpChoice
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton

/** Test tags of the answer buttons (S22, S37). */
object RsvpTags {
    fun answer(a: RsvpChoice) = "rsvp-${a.name.lowercase()}"
}

/**
 * T40: Yes, Maybe and No (S22's card and S37). The chosen answer is filled; the others are outlined. [busy] is the answer being sent
 * (its button waits). Screen readers hear which answer is chosen.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RsvpButtons(chosen: RsvpChoice?, busy: RsvpChoice?, onAnswer: (RsvpChoice) -> Unit, modifier: Modifier = Modifier) {
    FlowRow(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (a in RsvpChoice.entries) {
            val label = stringResource(
                when (a) {
                    RsvpChoice.YES -> R.string.invite_yes
                    RsvpChoice.MAYBE -> R.string.invite_maybe
                    RsvpChoice.NO -> R.string.invite_no
                },
            )
            val isChosen = a == chosen
            val described = if (isChosen) stringResource(R.string.invite_answer_chosen, label) else label
            CroftButton(
                label,
                { if (busy == null) onAnswer(a) },
                Modifier.testTag(RsvpTags.answer(a)).semantics { selected = isChosen; contentDescription = described },
                kind = if (isChosen) ButtonKind.FILLED else ButtonKind.OUTLINED,
                enabled = busy == null || busy == a,
                progress = if (busy == a) ButtonProgress.LOADING else ButtonProgress.IDLE,
            )
        }
    }
}
