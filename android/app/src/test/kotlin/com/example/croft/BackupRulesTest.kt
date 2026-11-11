// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element

/**
 * The mail database must not leave the phone through cloud backup or device-to-device transfer.
 * Plain XML checks on the source files (no Robolectric): the unit test working directory is `app/`.
 */
class BackupRulesTest {
    private val domains = setOf("root", "file", "database", "sharedpref", "external")
    private val android = "http://schemas.android.com/apk/res/android"

    private fun parse(path: String): Element {
        val f = File("src/main/$path")
        assertTrue("missing $f", f.isFile)
        val factory = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }
        return factory.newDocumentBuilder().parse(f).documentElement
    }

    private fun excluded(section: Element): Set<String> {
        val list = section.getElementsByTagName("exclude")
        return (0 until list.length).map { (list.item(it) as Element).getAttribute("domain") }.toSet()
    }

    @Test
    fun manifestDisablesBackupAndPointsToTheRules() {
        val app = parse("AndroidManifest.xml").getElementsByTagName("application").item(0) as Element
        assertEquals("false", app.getAttributeNS(android, "allowBackup"))
        assertEquals("@xml/data_extraction_rules", app.getAttributeNS(android, "dataExtractionRules"))
        assertEquals("@xml/backup_rules", app.getAttributeNS(android, "fullBackupContent"))
    }

    @Test
    fun dataExtractionRulesExcludeEverythingFromBackupAndTransfer() {
        val root = parse("res/xml/data_extraction_rules.xml")
        for (name in listOf("cloud-backup", "device-transfer")) {
            val sections = root.getElementsByTagName(name)
            assertEquals(name, 1, sections.length)
            assertEquals(name, domains, excluded(sections.item(0) as Element))
            assertEquals(name, 0, (sections.item(0) as Element).getElementsByTagName("include").length)
        }
    }

    @Test
    fun fullBackupContentExcludesEverything() {
        val root = parse("res/xml/backup_rules.xml")
        assertEquals(domains, excluded(root))
        assertEquals(0, root.getElementsByTagName("include").length)
    }
}
