package com.pcremote.stream

/** Pages of the stream menu. */
enum class MenuPage { MAIN, KEYS, MEDIA, DISPLAYS, RESOLUTION, STATS }

/** Rows of the main page, in the order they are shown. */
enum class MenuRow(val label: String) {
    MUTE("Mute audio"),
    KEYBOARD("Keyboard"),
    TRACKPAD("Trackpad"),
    PICTURE_IN_PICTURE("Picture in picture"),
    RESOLUTION("Resolution"),
    SWITCH_DISPLAY("Switch display"),
    STREAM_STATS("Stream stats"),
    MORE_KEYS("More keys"),
    HELP("Help"),
    DISCONNECT("Disconnect"),
}

object StreamMenuModel {
    const val DISCONNECT_HINT = "Ctrl+Alt+Q"
    const val AUDIO_OFF = "Host audio is off in settings"
    const val SHOW_STATS_BAR = "Show stats bar"
    const val AUDIO_HEADING = "Audio"
    const val AUDIO_COPY_DETAILS = "Copy details"
    const val AUDIO_REOPEN = "Reopen output"
    const val HELP_URL = "https://secpar.win/help/#stream-menu"
    const val WAITING_FOR_STATS = "Waiting for stream stats"

    /** The main page: Mute first (as on desktop, under its window toggle), Picture in picture only where the
     *  device has it, Switch display only with several displays. Clipboard sharing lives in Settings and the
     *  audio details on the Stream stats page. */
    fun mainRows(canPip: Boolean, displayCount: Int, audioOn: Boolean = true): List<MenuRow> = buildList {
        if (audioOn) add(MenuRow.MUTE)
        add(MenuRow.KEYBOARD)
        add(MenuRow.TRACKPAD)
        if (canPip) add(MenuRow.PICTURE_IN_PICTURE)
        add(MenuRow.RESOLUTION)
        if (displayCount > 1) add(MenuRow.SWITCH_DISPLAY)
        add(MenuRow.STREAM_STATS)
        add(MenuRow.MORE_KEYS)
        add(MenuRow.HELP)
        add(MenuRow.DISCONNECT)
    }

    /** The back row repeats the label of the row that opened the page. */
    fun backTitle(page: MenuPage): String = when (page) {
        MenuPage.MAIN -> ""
        MenuPage.KEYS -> MenuRow.MORE_KEYS.label
        MenuPage.MEDIA -> "Media keys"
        MenuPage.DISPLAYS -> MenuRow.SWITCH_DISPLAY.label
        MenuPage.RESOLUTION -> MenuRow.RESOLUTION.label
        MenuPage.STATS -> MenuRow.STREAM_STATS.label
    }

    /** Where Back goes. */
    fun backTarget(page: MenuPage): MenuPage = if (page == MenuPage.MEDIA) MenuPage.KEYS else MenuPage.MAIN

    /** Picking a size or a display returns to the main page and keeps the menu open. */
    fun pageAfterPick(): MenuPage = MenuPage.MAIN

    /** The strip text: the notice wins; stats come once there are some. */
    fun stripText(resizeNotice: Boolean, notice: String, perfText: String): String =
        if (resizeNotice) notice else perfText.ifEmpty { WAITING_FOR_STATS }

    /** Panel top edge in dp: below the strip when the strip shows. */
    fun panelTop(stripShown: Boolean): Int =
        if (stripShown) com.pcremote.ui.theme.MenuTokens.PANEL_Y_WITH_STRIP else com.pcremote.ui.theme.MenuTokens.PANEL_Y

    /** Text of the audio section on the Stream stats page; empty when the details come from the player. */
    fun audioBody(audioOn: Boolean, details: String): String = if (audioOn) details else AUDIO_OFF
}
