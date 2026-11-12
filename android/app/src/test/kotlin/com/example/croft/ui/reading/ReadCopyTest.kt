// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import com.example.croft.R
import com.example.croft.mail.reading.ReadProblem
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every `ReadError` the core can return has its own plain-language text (the same pattern as `FailureCopyTest`). The mapping from the
 * core's variants to [ReadProblem] is walked in `:mailtests` (`ReadingTest.everyVariantOfTheCoresReadErrorHasAProblem`); this walks the
 * problems to their strings: a new error in the core cannot slip through between the two. A plain JVM test: it reads
 * `strings_reading.xml` as a file.
 */
class ReadCopyTest {
    private val strings: Map<String, String> = Regex("""<string name="(\w+)">(.*?)</string>""", RegexOption.DOT_MATCHES_ALL)
        .findAll(File("src/main/res/values/strings_reading.xml").readText())
        .associate { it.groupValues[1] to it.groupValues[2] }

    @Test
    fun everyProblemHasItsOwnStringWithText() {
        assertEquals(17, ReadProblem.entries.size)
        for (problem in ReadProblem.entries) {
            val name = ReadCopy.resourceName(problem)
            assertEquals("$problem must use $name", R.string::class.java.getField(name).getInt(null), ReadCopy.resFor(problem))
            val text = strings[name]
            assertNotNull("strings_reading.xml has no <string name=\"$name\">", text)
            assertTrue("$name is empty", text!!.isNotBlank())
            assertTrue("$name shows a raw code: $text", !Regex("""Exception|0x[0-9a-f]+|\b[a-z]+_[a-z_]+\b|\b[A-Z]{2,}_[A-Z_]+\b""").containsMatchIn(text.replace(Regex("""%\d\$[sd]"""), "")))
        }
    }

    @Test
    fun theTextsAreDistinct_exceptThatOfflineIsTheOneTheSpecAsksFor() {
        val texts = ReadProblem.entries.map { strings.getValue(ReadCopy.resourceName(it)) }
        assertEquals(texts.size, texts.distinct().size)
        assertEquals("Not downloaded yet. It will load when you\\'re online.", strings.getValue(ReadCopy.resourceName(ReadProblem.OFFLINE)))
        assertEquals(strings.getValue("reading_not_downloaded"), strings.getValue("read_error_offline"))
    }

    @Test
    fun noStringOfTheReadingScreensIsEmptyOrNamesAnException() {
        for ((name, text) in strings) {
            assertTrue("$name is empty", text.isNotBlank())
            assertTrue("$name: $text", !text.contains("Exception"))
        }
    }
}
