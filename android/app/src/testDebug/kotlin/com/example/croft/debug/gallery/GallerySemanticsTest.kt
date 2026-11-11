// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug.gallery

import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.ROW_PREVIEW_TAG
import com.example.croft.ui.components.isInteractive
import com.example.croft.ui.components.settingsAt
import com.example.croft.ui.components.show
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Semantics of every gallery section at font scale 1.0 and 2.0: labels, disabled controls, touch targets, clipped text. */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class GallerySemanticsTest(private val sectionId: String, private val fontScale: Float) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-font{1}")
        fun cases(): List<Array<Any>> = GalleryCatalog.sections.flatMap { s -> listOf(1.0f, 2.0f).map { arrayOf<Any>(s.id, it) } }
    }

    @get:Rule
    val rule = createComposeRule()

    private val section get() = GalleryCatalog.sections.first { it.id == sectionId }

    private fun show() {
        RuntimeEnvironment.setQualifiers("+ldltr-w360dp-h4000dp-mdpi")
        RuntimeEnvironment.setFontScale(fontScale)
        rule.show(AnimationLevel.OFF) {
            androidx.compose.foundation.layout.Box(androidx.compose.ui.Modifier.verticalScroll(rememberScrollState())) {
                GallerySectionColumn(section, GalleryEnv(CroftSnackbarState()))
            }
        }
        rule.waitForIdle()
    }

    private fun interactive(): List<SemanticsNode> = rule.onAllNodes(isInteractive).fetchSemanticsNodes()

    private fun describe(n: SemanticsNode) =
        "${n.config.getOrNull(SemanticsProperties.Role)} text=${n.config.getOrNull(SemanticsProperties.Text)} desc=${n.config.getOrNull(SemanticsProperties.ContentDescription)} bounds=${n.touchBoundsInRoot}"

    @Test
    fun everySectionRendersAllItsEntries() {
        show()
        for (e in section.entries) rule.onNode(hasTestTag(galleryTag(e.id))).assertExists()
    }

    @Test
    fun everyInteractiveControlHasALabel_soIconOnlyControlsHaveAContentDescription() {
        show()
        val nodes = interactive()
        val unlabelled = nodes.filter { n ->
            val c = n.config
            val desc = c.getOrNull(SemanticsProperties.ContentDescription).orEmpty().any { it.isNotBlank() }
            val text = c.getOrNull(SemanticsProperties.Text).orEmpty().any { it.text.isNotBlank() }
            val editable = c.getOrNull(SemanticsProperties.EditableText)?.text?.isNotBlank() == true
            !(desc || text || editable)
        }
        assertTrue("${section.id}: controls without a label: ${unlabelled.map(::describe)}", unlabelled.isEmpty())
    }

    @Test
    fun everyInteractiveControlIsAtLeast48dpHighAndWide() {
        if (fontScale != 1.0f) return
        show()
        val min = rule.density.run { 48.dp.toPx() }
        val small = interactive().filter { n -> n.touchBoundsInRoot.width < min - 0.5f || n.touchBoundsInRoot.height < min - 0.5f }
        assertTrue("${section.id}: touch targets under 48 dp: ${small.map(::describe)}", small.isEmpty())
    }

    @Test
    fun disabledControlsAreInTheTreeAsDisabled() {
        if (fontScale != 1.0f) return
        show()
        val expectDisabled = mapOf(
            "common.disabled" to 1, "buttons.filled" to 1, "buttons.tonal" to 1, "buttons.outlined" to 1, "buttons.text" to 1, "buttons.danger" to 1,
            "send.no_recipient" to 1, "send.invalid_recipient" to 1, "send.over_size_limit" to 1, "send.sending" to 1,
            "chips.filter" to 1, "fields.disabled" to 1, "toggles.switch_disabled_on" to 1, "toggles.switch_disabled_off" to 1,
            "toggles.radio" to 1, "toggles.checkbox" to 1, "iconbutton.states" to 1,
        )
        for ((id, atLeast) in expectDisabled) {
            if (section.entries.none { it.id == id }) continue
            val disabled = rule.onAllNodes(hasAnyAncestor(hasTestTag(galleryTag(id))) and isInteractive and SemanticsMatcher.keyIsDefined(SemanticsProperties.Disabled)).fetchSemanticsNodes()
            assertTrue("$id: expected at least $atLeast disabled control(s) in the tree, found ${disabled.size}", disabled.size >= atLeast)
        }
    }

    /**
     * Whether some of the text is not visible: a line ends in an ellipsis, the lines beyond maxLines or the height are cut, or a
     * line is wider than the box it is drawn in. (`hasVisualOverflow` is also true for text that is merely narrower than the
     * width it was laid out for, so it cannot be used.)
     */
    private fun isClipped(r: TextLayoutResult): Boolean =
        (0 until r.lineCount).any { r.isLineEllipsized(it) || r.getLineRight(it) - r.getLineLeft(it) > r.size.width + 1f } ||
            r.multiParagraph.didExceedMaxLines || r.didOverflowHeight

    @Test
    fun atFontScale2_noTextIsClipped() {
        if (fontScale != 2.0f) return
        show()
        val clipped = ArrayList<String>()
        val nodes = rule.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult), useUnmergedTree = true).fetchSemanticsNodes()
        for (n in nodes) {
            if (n.config.getOrNull(SemanticsProperties.TestTag) == ROW_PREVIEW_TAG) continue
            val results = ArrayList<TextLayoutResult>()
            n.config[SemanticsActions.GetTextLayoutResult].action?.invoke(results)
            val r = results.firstOrNull() ?: continue
            if (isClipped(r)) clipped += "'${r.layoutInput.text.text}' (${n.boundsInRoot})"
        }
        assertTrue("${section.id}: clipped text at font scale 2.0: $clipped", clipped.isEmpty())
    }
}
