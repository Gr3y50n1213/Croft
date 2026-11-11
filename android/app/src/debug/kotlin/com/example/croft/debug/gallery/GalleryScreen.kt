// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug.gallery

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.clickable
import androidx.compose.ui.unit.dp
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.Haptics
import com.example.croft.haptics.LevelFilteredHaptics
import com.example.croft.haptics.LocalHaptics
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.LocalThemeSettings
import com.example.croft.theme.MotionLevel
import com.example.croft.theme.resolveMotionLevelForGallery
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState

/** Test tags of the gallery screen. */
object GalleryTags {
    const val SCREEN = "gallery"
    const val HAPTIC_LOG = "gallery-haptic-log"
    const val SECTION_PREFIX = "gallery-section-"
}

/**
 * The gallery: a list of sections, each showing every control of that kind in every state. Debug builds only. A row at the
 * top overrides the animation level for the gallery alone (to see the Reduced and Off substitutes), and a line at the
 * bottom shows the haptic events the controls asked for, marked when the current level would not play them.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GalleryScreen(onBack: () -> Unit, modifier: Modifier = Modifier) {
    var openSection by rememberSaveable { mutableStateOf<String?>(null) }
    var override by rememberSaveable { mutableStateOf<String?>(null) }
    BackHandler(enabled = openSection != null) { openSection = null }
    val snackbar = remember { CroftSnackbarState() }
    val log = remember { mutableStateListOf<String>() }
    val level = override?.let { runCatching { AnimationLevel.valueOf(it) }.getOrNull() }
    val section = GalleryCatalog.sections.firstOrNull { it.id == openSection }

    GalleryMotion(level) {
        val settings = LocalThemeSettings.current
        val real = LocalHaptics.current
        val logging = remember(real, settings.haptics) {
            object : Haptics {
                override fun perform(event: HapticEvent) {
                    val felt = LevelFilteredHaptics.allows(settings.haptics, event)
                    log.add(0, if (felt) event.name else event.name + " (not at ${settings.haptics.id})")
                    while (log.size > LOG_LINES) log.removeAt(log.lastIndex)
                    real.perform(event)
                }
            }
        }
        CompositionLocalProvider(LocalHaptics provides logging) {
            Scaffold(
                modifier = modifier.testTag(GalleryTags.SCREEN),
                containerColor = MaterialTheme.colorScheme.background,
                topBar = {
                    TopAppBar(
                        title = { Text(section?.title ?: "Component gallery") },
                        navigationIcon = {
                            IconButton(onClick = { if (section != null) openSection = null else onBack() }) {
                                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                            }
                        },
                    )
                },
                bottomBar = {
                    Column {
                        CroftSnackbarHost(snackbar)
                        Text(
                            "Haptics: " + (log.firstOrNull() ?: "none yet"),
                            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag(GalleryTags.HAPTIC_LOG),
                            style = MaterialTheme.typography.labelMedium,
                        )
                    }
                },
            ) { inner ->
                Column(Modifier.padding(inner).fillMaxSize()) {
                    Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text("Motion:", style = MaterialTheme.typography.labelMedium)
                        CroftFilterChip("App", override == null, { override = null })
                        AnimationLevel.entries.filter { it != AnimationLevel.SYSTEM }.forEach { l ->
                            CroftFilterChip(l.id.replaceFirstChar { it.uppercase() }, override == l.name, { override = l.name })
                        }
                    }
                    if (section == null) {
                        LazyColumn(Modifier.fillMaxSize()) {
                            items(GalleryCatalog.sections) { s ->
                                ListItem(
                                    headlineContent = { Text(s.title) },
                                    supportingContent = { Text("${s.entries.size} entries") },
                                    modifier = Modifier.testTag(GalleryTags.SECTION_PREFIX + s.id).clickable { openSection = s.id },
                                )
                            }
                        }
                    } else {
                        GallerySectionView(section, GalleryEnv(snackbar), Modifier.fillMaxSize())
                    }
                }
            }
        }
    }
}

private const val LOG_LINES = 5

/** Applies the gallery's own animation level, if one is chosen; otherwise the app's. */
@Composable
private fun GalleryMotion(level: AnimationLevel?, content: @Composable () -> Unit) {
    if (level == null) { content(); return }
    val resolved: MotionLevel = resolveMotionLevelForGallery(level)
    CompositionLocalProvider(LocalMotionLevel provides resolved) {
        androidx.compose.material3.MaterialTheme(motionScheme = com.example.croft.theme.motionSchemeFor(resolved), content = content)
    }
}

/**
 * Every entry of [section]: a caption and the control(s) in a bordered frame, with room around the frame for the focus ring
 * (3 dp and a 2 dp gap) so it is never clipped. Each entry carries the tag `gallery:<id>`.
 */
@Composable
fun GallerySectionView(section: GallerySection, env: GalleryEnv, modifier: Modifier = Modifier) {
    LazyColumn(modifier.testTag(GalleryTags.SECTION_PREFIX + section.id + "-list"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        items(section.entries, key = { it.id }) { entry ->
            GalleryTile(entry, env)
        }
    }
}

/** One entry as drawn in the gallery and in the screenshot tests. */
@Composable
fun GalleryTile(entry: GalleryEntry, env: GalleryEnv) {
    Column(Modifier.padding(horizontal = 16.dp).testTag(galleryTag(entry.id)), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(entry.label, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary, modifier = Modifier.semantics { heading() })
        Box(Modifier.fillMaxWidth().border(1.dp, MaterialTheme.colorScheme.outlineVariant, MaterialTheme.shapes.medium).padding(16.dp)) {
            with(entry) { env.content() }
        }
    }
}
