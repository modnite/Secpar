package com.pcremote.ui

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The strings file read as text: wording rules from docs/writing-style.md and the UI parity spec. */
class StringsTest {
    private val xml: String by lazy {
        val f = listOf("src/main/res/values/strings.xml", "app/src/main/res/values/strings.xml").map(::File).first { it.exists() }
        f.readText()
    }
    private val strings: Map<String, String> by lazy {
        Regex("""<string name="([^"]+)">(.*?)</string>""", RegexOption.DOT_MATCHES_ALL)
            .findAll(xml).associate { it.groupValues[1] to it.groupValues[2] }
    }

    @Test fun `no user text says PC or PC Remote`() {
        for ((name, text) in strings) {
            assertFalse("$name says PC: $text", Regex("""\bPCs?\b""").containsMatchIn(text))
        }
    }

    @Test fun `no text says Couldn't`() {
        for ((name, text) in strings) assertFalse("$name: $text", text.contains("Couldn\\'t") || text.contains("Couldn't"))
    }

    @Test fun `dead strings are gone`() {
        val gone = listOf("pcr_morning", "pcr_afternoon", "pcr_evening", "pcr_coach_title", "pcr_coach_edge", "pcr_coach_tap",
            "pcr_coach_pinch", "pcr_got_it", "pcr_hint_screen", "pcr_hint_remote", "pcr_hint_media", "pcr_hint_slides",
            "pcr_hint_pad", "pcr_hint_files", "pcr_ha_desc", "pcr_scan_hint", "pcr_tb_lower_quality",
            "pcr_dynamic_color", "pcr_dynamic_color_desc",
            "pcr_auto_quality", "pcr_auto_quality_desc", "pcr_preset_SMOOTH", "pcr_preset_SHARP", "pcr_preset_BATTERY",
            "pcr_natural_scroll_desc", "pcr_source_desc", "pcr_manual_desc", "pcr_no_pcs_body", "pcr_unlock_settings_desc",
            "pcr_unlock_settings_nofp", "secpar_wake_desc", "pcr_update_check_desc", "pcr_remove", "pcr_remove_confirm",
            "pcr_try_again", "pcr_version", "pcr_setup_guide", "pcr_set_quality", "pcr_pc_list_title")
        for (name in gone) assertFalse("$name still defined", strings.containsKey(name))
    }

    @Test fun `wake and connecting wording`() {
        assertEquals("Wake", strings["pcr_wake"])
        assertEquals("Connecting to %1\$s…", strings["pcr_connecting_to"])
        assertEquals("Could not start the screen", strings["pcr_s2_err_title"])
    }

    @Test fun `old code message does not name the old product`() {
        val text = strings["pcr_legacy_remote_pair"]
        assertNotNull(text)
        assertTrue(text!!.startsWith("This code is from an older version."))
        assertFalse(xml.contains("PC Remote"))
    }

    @Test fun `no setting row keeps a description string`() {
        assertEquals(emptyList<String>(), strings.keys.filter { it.endsWith("_desc") && it != "pcr_files_send_desc" })
    }

    @Test fun `pairing dialog and button wording`() {
        assertEquals("Pair a computer", strings["pcr_manual_title"])
        assertEquals("Pair and connect", strings["pcr_pair"])
        assertEquals("Paste a Secpar Host pairing link", strings["pcr_manual_hint"])
        assertEquals("Retry", strings["pcr_retry"])
        assertEquals("Pairing with %1\$s\u2026", strings["pcr_ob_pairing"])
    }

    @Test fun `home wording`() {
        assertEquals("Computers", strings["pcr_tab_computers"])
        assertEquals("Friends", strings["pcr_tab_friends"])
        assertEquals("Settings", strings["pcr_tab_settings"])
        assertEquals("Add computer", strings["pcr_add_pc"])
        assertEquals("No computers yet", strings["pcr_no_pcs_title"])
        assertEquals("Forget on this device", strings["pcr_forget_device"])
        assertEquals("Tap again to confirm", strings["pcr_pw_tap_again"])
        assertEquals("Updates", strings["pcr_updates"])
        assertEquals("Update available", strings["pcr_update_available"])
        assertEquals("Copy error", strings["pcr_copy_error"])
        assertEquals("Dismiss", strings["pcr_dismiss"])
    }

    @Test fun `no string uses three dots or an em dash`() {
        for ((name, text) in strings) {
            assertFalse("$name: $text", text.contains("..."))
            assertFalse("$name: $text", text.contains("\u2014"))
        }
    }

    @Test fun `onboarding keeps one short line on the second page only`() {
        assertEquals("Install Secpar on your computer.", strings["pcr_ob_get_body"])
        assertFalse(strings.containsKey("pcr_ob_welcome_body"))
        assertFalse(strings.containsKey("pcr_ob_scan_body"))
    }
}
