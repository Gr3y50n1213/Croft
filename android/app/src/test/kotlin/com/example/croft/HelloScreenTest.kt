// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The screen on a simulated device (Robolectric): no emulator needed. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class HelloScreenTest {
    @get:Rule
    val rule = createComposeRule()

    @Test
    fun showsTheNameAndBothVersions() {
        rule.setContent { CroftTheme { HelloScreen(RustInfo("9.8.7", 1u)) } }
        rule.onNodeWithText("Croft").assertIsDisplayed()
        rule.onNodeWithText("Core version 9.8.7").assertIsDisplayed()
        rule.onNodeWithText("FFI API version 1").assertIsDisplayed()
    }

    @Test
    fun showsAnErrorLineWhenTheCoreIsMissing() {
        rule.setContent { CroftTheme { HelloScreen(null) } }
        rule.onNodeWithText("The Croft core could not be loaded.").assertIsDisplayed()
    }
}

/** The real activity, calling the real Rust library (host build) through the generated bindings. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MainActivityTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    @Test
    fun showsTheVersionFromRust() {
        val expected = System.getProperty("croft.cargoVersion")
        rule.onNodeWithText("Core version $expected").assertIsDisplayed()
        rule.onNodeWithText("FFI API version 1").assertIsDisplayed()
    }
}
