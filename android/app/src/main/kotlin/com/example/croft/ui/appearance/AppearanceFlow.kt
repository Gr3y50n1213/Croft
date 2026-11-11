// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.LayoutDirection
import com.example.croft.R
import com.example.croft.design.CroftMotion
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import com.example.croft.theme.ThemeImportProblem
import com.example.croft.theme.ThemeImportResult
import com.example.croft.theme.ThemeJson
import com.example.croft.theme.ThemeSettings
import kotlinx.coroutines.launch

/** The four screens of the Appearance area. */
enum class AppearanceRoute { S50, S51, S52, S53 }

/**
 * S50 with its three sub-screens (S51, S52, S53) and the shared behaviour: Back returns to S50 first, a drill-down moves
 * sideways (shared axis X with the Spatial default spring; a plain fade at Reduced; instant at Off), Reset, Export theme
 * and Import theme. Stateless about the theme: the caller keeps [settings] (DataStore in the app, a variable in tests).
 */
@Composable
fun AppearanceFlow(
    settings: ThemeSettings,
    onChange: (ThemeSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    startAt: AppearanceRoute = AppearanceRoute.S50,
    onTry: (TryItEvent) -> Unit = {},
    files: ThemeFiles? = rememberThemeFiles(),
    photoPicker: PhotoSeedPicker? = rememberPhotoSeedPicker(),
) {
    var route by rememberSaveable { mutableStateOf(startAt) }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val level = LocalMotionLevel.current
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    BackHandler(enabled = route != AppearanceRoute.S50) { route = AppearanceRoute.S50 }

    fun say(text: String) { scope.launch { snackbar.currentSnackbarData?.dismiss(); snackbar.showSnackbar(text) } }

    AnimatedContent(
        targetState = route,
        modifier = modifier,
        label = "appearance-route",
        transitionSpec = {
            val forward = targetState != AppearanceRoute.S50
            val sign = if (forward != rtl) 1 else -1
            when (level) {
                MotionLevel.FULL -> {
                    val move: FiniteAnimationSpec<IntOffset> = CroftMotion.spatialDefault.spec()
                    val fade: FiniteAnimationSpec<Float> = CroftMotion.effectsDefault.spec()
                    (slideInHorizontally(move) { sign * it / 4 } + fadeIn(fade)) togetherWith
                        (slideOutHorizontally(move) { -sign * it / 4 } + fadeOut(fade))
                }
                MotionLevel.REDUCED -> fadeIn(CroftMotion.effectsDefault.spec()) togetherWith fadeOut(CroftMotion.effectsDefault.spec())
                MotionLevel.OFF -> EnterTransition.None togetherWith ExitTransition.None
            }
        },
    ) { current ->
        when (current) {
            AppearanceRoute.S50 -> AppearanceScreen(
                settings = settings,
                onChange = onChange,
                onBack = onBack,
                onOpenCustomColour = { route = AppearanceRoute.S51 },
                onOpenLayout = { route = AppearanceRoute.S52 },
                onOpenMotion = { route = AppearanceRoute.S53 },
                onReset = { onChange(ThemeSettings.Default); say(context.getString(R.string.appearance_reset_done)) },
                snackbarHostState = snackbar,
                onExportTheme = files?.let { f ->
                    { f.export(ThemeJson.encode(settings)) { ok -> if (ok) say(context.getString(R.string.appearance_exported)) } }
                },
                onImportTheme = files?.let { f ->
                    {
                        f.import { text ->
                            if (text != null) when (val r = ThemeJson.decode(text)) {
                                is ThemeImportResult.Imported -> { onChange(r.settings); say(context.getString(R.string.appearance_imported)) }
                                is ThemeImportResult.Rejected -> say(context.getString(importProblemRes(r.problem)))
                            } else say(context.getString(R.string.appearance_import_unreadable))
                        }
                    }
                },
            )
            AppearanceRoute.S51 -> CustomColourScreen(settings, onApply = onChange, onBack = { route = AppearanceRoute.S50 }, photoPicker = photoPicker)
            AppearanceRoute.S52 -> LayoutDensityScreen(settings, onChange, onBack = { route = AppearanceRoute.S50 })
            AppearanceRoute.S53 -> MotionHapticsScreen(settings, onChange, onBack = { route = AppearanceRoute.S50 }, onTry = onTry)
        }
    }
}

internal fun importProblemRes(problem: ThemeImportProblem): Int = when (problem) {
    ThemeImportProblem.EMPTY -> R.string.appearance_import_empty
    ThemeImportProblem.TOO_BIG -> R.string.appearance_import_too_big
    ThemeImportProblem.NOT_JSON -> R.string.appearance_import_not_json
    ThemeImportProblem.NOT_A_THEME -> R.string.appearance_import_not_a_theme
    ThemeImportProblem.TOO_NEW -> R.string.appearance_import_too_new
}
