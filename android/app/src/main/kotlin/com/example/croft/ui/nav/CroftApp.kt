// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.nav

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import com.example.croft.CroftTheme
import com.example.croft.RustInfo
import com.example.croft.mail.MailSession
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStore
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * The app as the activity shows it: the stored theme applied with [CroftTheme], and the navigation inside it. Nothing is
 * drawn until the stored settings have been read, so the app never flashes the default look first (the window background
 * of the activity theme shows meanwhile). A change made in Appearance shows at once and is written behind it; DataStore
 * keeps the writes in order, so the last one wins.
 */
@Composable
fun CroftApp(store: ThemeStore, info: RustInfo?, session: MailSession, modifier: Modifier = Modifier, startAt: Route = Route.HOME) {
    var settings by remember { mutableStateOf<ThemeSettings?>(null) }
    LaunchedEffect(store) { settings = store.settings.first() }
    val scope = rememberCoroutineScope()
    settings?.let { current ->
        CroftTheme(current) {
            CroftNavHost(
                info = info,
                session = session,
                settings = current,
                modifier = modifier,
                startAt = startAt,
                onSettingsChange = { next -> settings = next; scope.launch { store.set(next) } },
            )
        }
    }
}
