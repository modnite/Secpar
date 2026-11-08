package com.pcremote.stream

import com.pcremote.stream2.RESIZE_UNSUPPORTED
import com.pcremote.ui.theme.MenuTokens
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StreamMenuModelTest {
    @Test fun `main page order with every optional row`() {
        assertEquals(
            listOf("Keyboard", "Trackpad", "Picture in picture", "Resolution", "Switch display", "Share clipboard",
                "Stream stats", "Audio", "More keys", "Help", "Disconnect"),
            StreamMenuModel.mainRows(canPip = true, displayCount = 2).map { it.label })
    }

    @Test fun `picture in picture and switch display are optional`() {
        val rows = StreamMenuModel.mainRows(canPip = false, displayCount = 1)
        assertFalse(MenuRow.PICTURE_IN_PICTURE in rows)
        assertFalse(MenuRow.SWITCH_DISPLAY in rows)
        assertEquals(MenuRow.DISCONNECT, rows.last())
        assertTrue(MenuRow.SWITCH_DISPLAY in StreamMenuModel.mainRows(false, 2))
        assertFalse(MenuRow.SWITCH_DISPLAY in StreamMenuModel.mainRows(false, 0))
    }

    @Test fun `share clipboard sits before stream stats and audio after it`() {
        val rows = StreamMenuModel.mainRows(true, 3)
        assertEquals(rows.indexOf(MenuRow.STREAM_STATS) - 1, rows.indexOf(MenuRow.SHARE_CLIPBOARD))
        assertEquals(rows.indexOf(MenuRow.STREAM_STATS) + 1, rows.indexOf(MenuRow.AUDIO))
    }

    @Test fun `back row title is the label of the row that opened the page`() {
        assertEquals("More keys", StreamMenuModel.backTitle(MenuPage.KEYS))
        assertEquals("Switch display", StreamMenuModel.backTitle(MenuPage.DISPLAYS))
        assertEquals("Resolution", StreamMenuModel.backTitle(MenuPage.RESOLUTION))
        assertEquals("Audio", StreamMenuModel.backTitle(MenuPage.AUDIO))
        assertEquals("Media keys", StreamMenuModel.backTitle(MenuPage.MEDIA))
    }

    @Test fun `back goes up one level and a pick returns to the main page`() {
        assertEquals(MenuPage.KEYS, StreamMenuModel.backTarget(MenuPage.MEDIA))
        for (p in listOf(MenuPage.KEYS, MenuPage.DISPLAYS, MenuPage.RESOLUTION, MenuPage.AUDIO))
            assertEquals(MenuPage.MAIN, StreamMenuModel.backTarget(p))
        assertEquals(MenuPage.MAIN, StreamMenuModel.pageAfterPick())
    }

    @Test fun `disconnect shows its shortcut`() {
        assertEquals("Ctrl+Alt+Q", StreamMenuModel.DISCONNECT_HINT)
    }

    @Test fun `strip waits for stats and the notice wins`() {
        assertEquals("Waiting for stream stats", StreamMenuModel.stripText(false, RESIZE_UNSUPPORTED, ""))
        assertEquals("1280 × 720", StreamMenuModel.stripText(false, RESIZE_UNSUPPORTED, "1280 × 720"))
        assertEquals(RESIZE_UNSUPPORTED, StreamMenuModel.stripText(true, RESIZE_UNSUPPORTED, "1280 × 720"))
    }

    @Test fun `audio page text`() {
        assertEquals("Host audio is off in settings", StreamMenuModel.audioBody(false, "Output: x"))
        assertEquals("Output: x", StreamMenuModel.audioBody(true, "Output: x"))
        assertEquals("Copy details", StreamMenuModel.AUDIO_COPY_DETAILS)
        assertEquals("Reopen output", StreamMenuModel.AUDIO_REOPEN)
    }

    @Test fun `panel sits 60 across and 12 down or 34 with the strip`() {
        assertEquals(60, MenuTokens.PANEL_X)
        assertEquals(12, StreamMenuModel.panelTop(false))
        assertEquals(34, StreamMenuModel.panelTop(true))
    }

    @Test fun `geometry matches the desktop menu`() {
        assertEquals(40, MenuTokens.BUTTON); assertEquals(24, MenuTokens.LOGO)
        assertEquals(262, MenuTokens.PANEL_WIDTH); assertEquals(6, MenuTokens.PANEL_PADDING)
        assertEquals(34, MenuTokens.ROW_HEIGHT); assertEquals(20, MenuTokens.ICON)
        assertEquals(28, MenuTokens.STRIP_HEIGHT)
        // icon at x+9, label at x+39: 10 between them; panel content is 250 wide
        assertEquals(10, MenuTokens.LABEL_X - MenuTokens.ICON_X - MenuTokens.ICON)
        assertEquals(250, MenuTokens.PANEL_WIDTH - 2 * MenuTokens.PANEL_PADDING)
        // a check or chevron is 18 wide, centred 15 from the right edge: 6 from the edge, 6 clear of the value
        assertEquals(6, MenuTokens.TRAILING_CENTRE - MenuTokens.TRAILING_ICON / 2)
        assertEquals(6, MenuTokens.VALUE_END_WITH_TRAILING - MenuTokens.TRAILING_ICON - (MenuTokens.TRAILING_CENTRE - MenuTokens.TRAILING_ICON / 2))
        // footer text starts 15 from the panel edge, in line with the icons (6 padding + 9)
        assertEquals(MenuTokens.FOOTER_X, MenuTokens.PANEL_PADDING + MenuTokens.ICON_X)
    }

    @Test fun `colours are the desktop menu colours`() {
        assertEquals(0xF6191B1D, MenuTokens.FILL)
        assertEquals(246L, MenuTokens.FILL ushr 24)
        assertEquals(135L, MenuTokens.BUTTON_FILL ushr 24)
        assertEquals(215L, MenuTokens.STRIP_FILL ushr 24)
        assertEquals(0xFFE9EBEDL, MenuTokens.TEXT)
        assertEquals(0xFF94999EL, MenuTokens.MUTED)
        assertEquals(0xFFF2695EL, MenuTokens.DANGER)
        assertEquals(0xFF0AFFB6L, MenuTokens.CHECK)
        assertEquals(12, MenuTokens.ROW_PRESSED_ALPHA); assertEquals(23, MenuTokens.DIVIDER_ALPHA)
    }
}
