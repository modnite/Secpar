package com.pcremote.modes

import android.view.KeyEvent
import com.pcremote.agent.Msg
import kotlin.math.hypot

/** One physical controller's axes, already chosen per device: sticks -1..1 (down = +y), triggers 0..1, hat -1/0/1. */
data class PadAxes(
    val lx: Float = 0f, val ly: Float = 0f, val rx: Float = 0f, val ry: Float = 0f,
    val lt: Float = 0f, val rt: Float = 0f, val hatX: Int = 0, val hatY: Int = 0,
)

/**
 * Physical controllers (Bluetooth, USB-OTG) as `pad` messages. Each device takes the lowest free
 * pad id from [firstId] to 3 and gives it back when unplugged. Only changes are sent, at most
 * every 8 ms. A held state is refreshed every 250 ms and a change is repeated twice quickly,
 * because input travels over UDP and the host only keeps the latest state.
 */
class PhysicalPads(private val send: (Msg) -> Unit, private val firstId: Int = 0) {
    private class Slot(val device: Int) {
        var keys = 0
        var l2 = false
        var r2 = false
        var axes = PadAxes()
        var last: PadState? = null
        var lastAt = Long.MIN_VALUE / 2
        var quick = 0
    }

    private val slots = arrayOfNulls<Slot>(MAX_ID + 1 - firstId)

    val count get() = slots.count { it != null }

    private fun slot(device: Int): Slot? {
        slots.firstOrNull { it?.device == device }?.let { return it }
        val free = slots.indexOfFirst { it == null }
        if (free < 0) return null
        return Slot(device).also { slots[free] = it }
    }

    fun idOf(device: Int): Int? = slots.indexOfFirst { it?.device == device }.takeIf { it >= 0 }?.plus(firstId)
    fun deviceOf(id: Int): Int? = slots.getOrNull(id - firstId)?.device

    /** True when the key belongs to a controller button and was taken. */
    fun key(device: Int, keyCode: Int, down: Boolean): Boolean {
        val mask = buttonMask(keyCode)
        val l2 = keyCode == KeyEvent.KEYCODE_BUTTON_L2
        val r2 = keyCode == KeyEvent.KEYCODE_BUTTON_R2
        if (mask == 0 && !l2 && !r2) return false
        val s = slot(device) ?: return true
        if (mask != 0) s.keys = if (down) s.keys or mask else s.keys and mask.inv()
        if (l2) s.l2 = down
        if (r2) s.r2 = down
        return true
    }

    fun axes(device: Int, a: PadAxes): Boolean {
        val s = slot(device) ?: return true
        s.axes = a
        return true
    }

    /** The state the host should see for this slot. */
    private fun state(s: Slot): PadState {
        var st = PadState(btn = s.keys)
        val a = s.axes
        if (a.hatX < 0) st = st.press(PadButton.LEFT) else if (a.hatX > 0) st = st.press(PadButton.RIGHT)
        if (a.hatY < 0) st = st.press(PadButton.UP) else if (a.hatY > 0) st = st.press(PadButton.DOWN)
        val (lx, ly) = deadZone(a.lx, a.ly)
        val (rx, ry) = deadZone(a.rx, a.ry)
        st = st.stick(true, lx, ly).stick(false, rx, ry)
        val lt = if (a.lt > TRIGGER_DEAD) a.lt else if (s.l2) 1f else 0f
        val rt = if (a.rt > TRIGGER_DEAD) a.rt else if (s.r2) 1f else 0f
        return st.trigger(true, lt).trigger(false, rt)
    }

    /** Call every few milliseconds. [now] is a monotonic clock in ms. */
    fun flush(now: Long) {
        slots.forEachIndexed { i, s ->
            if (s == null) return@forEachIndexed
            val st = state(s)
            val gap = now - s.lastAt
            val changed = st != s.last
            val due = when {
                changed -> gap >= MIN_GAP_MS
                s.quick > 0 -> gap >= QUICK_GAP_MS
                else -> st != PadState() && gap >= HELD_GAP_MS
            }
            if (!due) return@forEachIndexed
            s.quick = if (changed) 2 else s.quick - 1
            s.last = st
            s.lastAt = now
            send(st.toMsg(i + firstId))
        }
    }

    /** Unplugged: the host drops that controller. */
    fun removed(device: Int) {
        val i = slots.indexOfFirst { it?.device == device }
        if (i < 0) return
        slots[i] = null
        send(Msg.PadOff(i + firstId))
    }

    /** Focus lost or the screen closed: nothing stays held on the host. */
    fun releaseAll() {
        slots.forEachIndexed { i, s ->
            if (s != null) { slots[i] = null; send(Msg.PadOff(i + firstId)) }
        }
    }

    companion object {
        const val MAX_ID = 3
        private const val MIN_GAP_MS = 8L
        private const val QUICK_GAP_MS = 40L
        private const val HELD_GAP_MS = 250L
        private const val STICK_DEAD = 0.08f
        private const val TRIGGER_DEAD = 0.02f

        /** Radial dead zone that still reaches full deflection. */
        fun deadZone(x: Float, y: Float, dead: Float = STICK_DEAD): Pair<Float, Float> {
            val m = hypot(x, y)
            if (m <= dead) return 0f to 0f
            if (m >= 1f) return x to y // a full push, even into a square corner, is left alone
            val k = ((m - dead) / (1f - dead)).coerceAtMost(1f) / m
            return x * k to y * k
        }

        fun isPadKey(keyCode: Int) = buttonMask(keyCode) != 0 || keyCode == KeyEvent.KEYCODE_BUTTON_L2 || keyCode == KeyEvent.KEYCODE_BUTTON_R2

        fun buttonMask(keyCode: Int): Int = when (keyCode) {
            KeyEvent.KEYCODE_BUTTON_A -> PadButton.A.mask
            KeyEvent.KEYCODE_BUTTON_B -> PadButton.B.mask
            KeyEvent.KEYCODE_BUTTON_X -> PadButton.X.mask
            KeyEvent.KEYCODE_BUTTON_Y -> PadButton.Y.mask
            KeyEvent.KEYCODE_BUTTON_L1 -> PadButton.LB.mask
            KeyEvent.KEYCODE_BUTTON_R1 -> PadButton.RB.mask
            KeyEvent.KEYCODE_BUTTON_THUMBL -> PadButton.LS.mask
            KeyEvent.KEYCODE_BUTTON_THUMBR -> PadButton.RS.mask
            KeyEvent.KEYCODE_BUTTON_START -> PadButton.START.mask
            KeyEvent.KEYCODE_BUTTON_SELECT -> PadButton.BACK.mask
            KeyEvent.KEYCODE_BUTTON_MODE -> PadButton.GUIDE.mask
            KeyEvent.KEYCODE_DPAD_UP -> PadButton.UP.mask
            KeyEvent.KEYCODE_DPAD_DOWN -> PadButton.DOWN.mask
            KeyEvent.KEYCODE_DPAD_LEFT -> PadButton.LEFT.mask
            KeyEvent.KEYCODE_DPAD_RIGHT -> PadButton.RIGHT.mask
            else -> 0
        }
    }
}
