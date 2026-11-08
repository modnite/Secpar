package com.pcremote.stream

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

enum class LinkState { GOOD, POOR, RECONNECTING }

/** What the stream overlay shows; the Screen mode updates it. */
class ToolbarState {
    var link by mutableStateOf(LinkState.GOOD)
    var perfText by mutableStateOf("")
    var showPerf by mutableStateOf(false)
    var trackpad by mutableStateOf(true)
    var keyboardOpen by mutableStateOf(false)
    var connecting by mutableStateOf<String?>(null)
    var mouseCaptured by mutableStateOf(false)
    var canCaptureMouse by mutableStateOf(true)
    var canPip by mutableStateOf(false)
    var connected by mutableStateOf(false)
    var pcName by mutableStateOf<String?>(null)
    /** The computer's displays (only offered when there are several) and the one being streamed. */
    var displays by mutableStateOf<List<com.pcremote.agent.DisplayInfo>>(emptyList())
    var display by mutableStateOf("")
    /** The size last picked in the menu (0 x 0 = the computer display's own), and whether the old-host notice shows. */
    var sizeW by mutableStateOf(com.pcremote.settings.SettingsModel.DEFAULT_RESOLUTION.first)
    var sizeH by mutableStateOf(com.pcremote.settings.SettingsModel.DEFAULT_RESOLUTION.second)
    /** "Share clipboard" row (saved in settings, default on) and whether host audio is on. */
    var shareClipboard by mutableStateOf(true)
    var audioOn by mutableStateOf(true)
    var resizeNotice by mutableStateOf(false)
    /** Ctrl+Alt+S hides the Secpar button. */
    var buttonHidden by mutableStateOf(false)
    val bar = KeyboardBarModel()
    var barTick by mutableIntStateOf(0)
}

interface ToolbarActions {
    fun shortcut(s: Shortcut)
    fun keyboard()
    fun setTrackpad(on: Boolean)
    fun disconnect()
    fun keys(events: List<KeyEvt>)
    fun captureMouse() {}
    fun pictureInPicture() {}
    fun selectDisplay(id: String)
    fun selectSize(w: Int, h: Int) {}
    fun setShareClipboard(on: Boolean) {}
    /** The Audio page body: output device and playback counters. */
    fun audioDetails(): String = ""
    fun copyText(text: String) {}
    fun reopenAudio() {}
}
