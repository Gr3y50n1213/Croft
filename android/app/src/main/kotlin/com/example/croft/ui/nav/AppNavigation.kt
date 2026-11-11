// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.nav

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import com.example.croft.HelloScreen
import com.example.croft.RustInfo
import com.example.croft.debug.DebugGallery
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.appearance.AppearanceFlow
import com.example.croft.ui.components.sharedAxisX

/** The screens reachable today. S50 and its drill-down live inside [APPEARANCE] (see `AppearanceFlow`). */
enum class Route(val parent: Route?) {
    /** The start screen. */
    HOME(null),

    /** The settings list. */
    SETTINGS(HOME),

    /** S50 Appearance and its sub-screens. */
    APPEARANCE(SETTINGS),

    /** The component gallery (debug builds only). */
    GALLERY(SETTINGS),
    ;

    /** How deep the screen is; going to a larger depth is "forward". */
    val depth: Int get() = generateSequence(this) { it.parent }.count() - 1
}

/**
 * The app's simple navigation: Home, Settings, Appearance, and (in debug builds) the component gallery. Back goes to the
 * parent screen through the system back-callback API (`BackHandler` registers an `OnBackPressedCallback`); on Home it is
 * left to the system. Moving between screens uses the shared axis X transition at the animation level in force. The
 * route survives rotation and process death.
 */
@Composable
fun CroftNavHost(
    info: RustInfo?,
    settings: ThemeSettings,
    onSettingsChange: (ThemeSettings) -> Unit,
    modifier: Modifier = Modifier,
    startAt: Route = Route.HOME,
) {
    var route by rememberSaveable { mutableStateOf(startAt) }
    // A debug-only route can never be reached from a release build, even from saved state.
    if (route == Route.GALLERY && !DebugGallery.available) route = Route.SETTINGS
    BackHandler(enabled = route != Route.HOME) { route = route.parent ?: Route.HOME }
    val level = LocalMotionLevel.current
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    AnimatedContent(
        targetState = route,
        modifier = modifier,
        label = "app-route",
        transitionSpec = { sharedAxisX(level, forward = targetState.depth > initialState.depth, rtl = rtl) },
    ) { current ->
        when (current) {
            Route.HOME -> HelloScreen(info, onOpenSettings = { route = Route.SETTINGS })
            Route.SETTINGS -> SettingsHomeScreen(
                onOpenAppearance = { route = Route.APPEARANCE },
                onOpenGallery = if (DebugGallery.available) ({ route = Route.GALLERY }) else null,
                onBack = { route = Route.HOME },
            )
            Route.APPEARANCE -> AppearanceFlow(settings = settings, onChange = onSettingsChange, onBack = { route = Route.SETTINGS })
            Route.GALLERY -> DebugGallery.Screen(onBack = { route = Route.SETTINGS })
        }
    }
}
