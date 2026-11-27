package com.pdflector.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Element

class BackupRulesTest {
    private val mainDir = File("src/main")

    private fun parseXml(path: String): Element {
        val file = File(mainDir, path)
        assertTrue("Expected XML file to exist: ${file.path}", file.isFile)
        val factory = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }
        return factory.newDocumentBuilder().parse(file).documentElement
    }

    @Test
    fun manifestWiresBackupAndDataExtractionRulesAndKeepsBackupEnabled() {
        val root = parseXml("AndroidManifest.xml")
        val app = root.getElementsByTagName("application").item(0) as Element
        val ns = "http://schemas.android.com/apk/res/android"
        assertFalse(
            "Backup should stay enabled so notebooks remain backed up",
            app.getAttributeNS(ns, "allowBackup") == "false",
        )
        assertTrue(
            "Manifest must declare android:fullBackupContent=\"@xml/backup_rules\"",
            app.getAttributeNS(ns, "fullBackupContent") == "@xml/backup_rules",
        )
        assertTrue(
            "Manifest must declare android:dataExtractionRules=\"@xml/data_extraction_rules\"",
            app.getAttributeNS(ns, "dataExtractionRules") == "@xml/data_extraction_rules",
        )
    }

    @Test
    fun backupRulesExcludeGeminiKeyOnly() {
        val root = parseXml("res/xml/backup_rules.xml")
        val excludes = root.getElementsByTagName("exclude")
        val excludedPaths = (0 until excludes.length).map { i ->
            val el = excludes.item(i) as Element
            el.getAttribute("domain") to el.getAttribute("path")
        }
        assertTrue(
            "backup_rules.xml must exclude file/gemini_api_key",
            ("file" to "gemini_api_key") in excludedPaths,
        )
        assertFalse(
            "backup_rules.xml must not exclude notebooks.json",
            excludedPaths.any { it.second.contains("notebooks") },
        )
    }

    @Test
    fun dataExtractionRulesExcludeGeminiKeyFromCloudAndDeviceTransfer() {
        val root = parseXml("res/xml/data_extraction_rules.xml")
        for (section in listOf("cloud-backup", "device-transfer")) {
            val nodes = root.getElementsByTagName(section)
            assertTrue("Missing <$section> in data_extraction_rules.xml", nodes.length == 1)
            val excludes = (nodes.item(0) as Element).getElementsByTagName("exclude")
            val excludedPaths = (0 until excludes.length).map { i ->
                val el = excludes.item(i) as Element
                el.getAttribute("domain") to el.getAttribute("path")
            }
            assertTrue(
                "<$section> must exclude file/gemini_api_key",
                ("file" to "gemini_api_key") in excludedPaths,
            )
            assertFalse(
                "<$section> must not exclude notebooks.json",
                excludedPaths.any { it.second.contains("notebooks") },
            )
        }
    }
}
