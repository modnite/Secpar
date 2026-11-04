package com.pcremote.stream2

import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import com.pcremote.agent.Msg
import kotlin.math.roundToInt

/** Physical input is independent of the optional on-screen typing field. */
class DesktopInput(private val send: (Msg) -> Unit) {
    private val keys = mutableSetOf<Int>()
    private var buttons = 0
    private var dx = 0f
    private var dy = 0f

    fun key(e: KeyEvent): Boolean {
        if (!e.isFromSource(InputDevice.SOURCE_KEYBOARD) || e.device?.isVirtual != false) return false
        val vk = virtualKey(e.keyCode) ?: return false
        when (e.action) {
            KeyEvent.ACTION_DOWN -> { keys.add(vk); send(Msg.Key(vk, true)) }
            KeyEvent.ACTION_UP -> { keys.remove(vk); send(Msg.Key(vk, false)) }
            else -> return false
        }
        return true
    }

    fun mouse(e: MotionEvent): Boolean {
        if (!e.isFromSource(InputDevice.SOURCE_MOUSE) && !e.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE) && !e.isFromSource(InputDevice.SOURCE_TOUCHPAD)) return false
        if (e.actionMasked == MotionEvent.ACTION_MOVE) {
            val ax = if (e.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE)) MotionEvent.AXIS_X else MotionEvent.AXIS_RELATIVE_X
            val ay = if (e.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE)) MotionEvent.AXIS_Y else MotionEvent.AXIS_RELATIVE_Y
            dx += e.getAxisValue(ax); dy += e.getAxisValue(ay)
            for (i in 0 until e.historySize) {
                dx += e.getHistoricalAxisValue(ax, i); dy += e.getHistoricalAxisValue(ay, i)
            }
            val x = dx.toInt(); val y = dy.toInt()
            dx -= x; dy -= y
            if (x != 0 || y != 0) send(Msg.MouseMove(x, y))
        }
        if (e.actionMasked == MotionEvent.ACTION_SCROLL) {
            send(Msg.MouseScroll((e.getAxisValue(MotionEvent.AXIS_VSCROLL) * 120).roundToInt(),
                (e.getAxisValue(MotionEvent.AXIS_HSCROLL) * 120).roundToInt()))
        }
        for ((mask, name) in buttonMap) {
            if ((buttons and mask) != (e.buttonState and mask)) send(Msg.MouseButton(name, e.buttonState and mask != 0))
        }
        buttons = e.buttonState
        return true
    }

    fun release() {
        keys.toList().forEach { send(Msg.Key(it, false)) }; keys.clear()
        for ((mask, name) in buttonMap) if (buttons and mask != 0) send(Msg.MouseButton(name, false))
        buttons = 0; dx = 0f; dy = 0f
    }

    companion object {
        private val buttonMap = listOf(MotionEvent.BUTTON_PRIMARY to "left", MotionEvent.BUTTON_SECONDARY to "right", MotionEvent.BUTTON_TERTIARY to "middle")
        internal fun virtualKey(k: Int): Int? = when (k) {
            in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> 0x41 + k - KeyEvent.KEYCODE_A
            in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 -> 0x30 + k - KeyEvent.KEYCODE_0
            in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> 0x70 + k - KeyEvent.KEYCODE_F1
            in KeyEvent.KEYCODE_NUMPAD_0..KeyEvent.KEYCODE_NUMPAD_9 -> 0x60 + k - KeyEvent.KEYCODE_NUMPAD_0
            else -> mapOf(4 to 0x1B, 111 to 0x1B, 61 to 9, 62 to 32, 66 to 13, 67 to 8,
                112 to 0x2E, 124 to 0x2D, 122 to 0x24, 123 to 0x23, 92 to 0x21, 93 to 0x22,
                19 to 0x26, 20 to 0x28, 21 to 0x25, 22 to 0x27,
                59 to 0xA0, 60 to 0xA1, 113 to 0xA2, 114 to 0xA3, 57 to 0xA4, 58 to 0xA5,
                117 to 0x5B, 118 to 0x5C, 115 to 0x14, 116 to 0x91, 143 to 0x90,
                55 to 0xBC, 56 to 0xBE, 69 to 0xBD, 70 to 0xBB, 71 to 0xDB, 72 to 0xDD,
                73 to 0xDC, 74 to 0xBA, 75 to 0xDE, 76 to 0xBF, 68 to 0xC0,
                154 to 0x6F, 155 to 0x6A, 156 to 0x6D, 157 to 0x6B, 158 to 0x6E, 160 to 13,
                120 to 0x2C, 121 to 0x13)[k]
        }
    }
}
