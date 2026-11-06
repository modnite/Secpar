package com.pcremote.stream2

/** Explicit user choice outlives pointer capture and focus callbacks. */
class DesktopCapture(private val releaseInput: () -> Unit, private val nativeCapture: (Boolean) -> Unit) {
    var requested = false; private set
    private var uiOpen = false
    val forwarding get() = requested && !uiOpen

    fun setRequested(on: Boolean) {
        requested = on
        refresh()
    }
    fun toggle() = setRequested(!requested)
    fun setUiOpen(open: Boolean) {
        uiOpen = open
        if (open) requested = false
        refresh()
    }
    fun refresh() {
        if (!forwarding) releaseInput()
        nativeCapture(forwarding)
    }
    fun lostFocus() = setRequested(false)
}

enum class CaptureShortcut(val label: String) {
    CTRL_ALT_G("Ctrl+Alt+G"), CTRL_SHIFT_G("Ctrl+Shift+G"), CTRL_ALT_Z("Ctrl+Alt+Z"),
    CTRL_F12("Ctrl+F12"), CTRL_SHIFT_F12("Ctrl+Shift+F12"), MOONLIGHT("Ctrl+Alt+Shift+Z");
    fun matches(key: Int, ctrl: Boolean, alt: Boolean, shift: Boolean, meta: Boolean): Boolean {
        if (!ctrl || meta) return false
        return when (this) {
            CTRL_ALT_G -> key == 35 && alt && !shift
            CTRL_SHIFT_G -> key == 35 && shift && !alt
            CTRL_ALT_Z -> key == 54 && alt && !shift
            CTRL_F12 -> key == 142 && !alt && !shift
            CTRL_SHIFT_F12 -> key == 142 && !alt && shift
            MOONLIGHT -> key == 54 && alt && shift
        }
    }
}
