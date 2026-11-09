package com.pcremote.guest

import android.view.KeyEvent
import android.view.MotionEvent
import com.pcremote.agent.Msg
import com.pcremote.modes.PadButton
import com.pcremote.modes.PadState
import kotlin.math.hypot
import kotlin.math.max

/**
 * One local controller as the XInput layout. The Android input framework has already turned Xbox,
 * DualShock 4, DualSense, Switch Pro and 8BitDo pads into KEYCODE_BUTTON_* and AXIS_* values. What is
 * left is that some pads put the right stick on Z/RZ and the triggers on RX/RY, or on BRAKE/GAS, so the
 * layout is chosen from the axes the device reports. [ranges] maps each reported axis to its (min, max).
 */
class PadMapper(private val ranges: Map<Int, Pair<Float, Float>>) {
    private val ltAxis: Int?
    private val rtAxis: Int?
    private val rxAxis: Int
    private val ryAxis: Int
    private val keys = HashSet<Int>()
    private var values = HashMap<Int, Float>()

    init {
        fun has(a: Int) = a in ranges
        var lt = listOf(MotionEvent.AXIS_LTRIGGER, MotionEvent.AXIS_BRAKE).firstOrNull(::has)
        var rt = listOf(MotionEvent.AXIS_RTRIGGER, MotionEvent.AXIS_GAS).firstOrNull(::has)
        val zr = has(MotionEvent.AXIS_Z) && has(MotionEvent.AXIS_RZ)
        val rxry = has(MotionEvent.AXIS_RX) && has(MotionEvent.AXIS_RY)
        var rx = MotionEvent.AXIS_Z
        var ry = MotionEvent.AXIS_RZ
        if (lt == null && rt == null && zr && rxry) {
            if ((ranges[MotionEvent.AXIS_Z]?.first ?: -1f) >= 0f) {
                // Z and RZ rest at 0: triggers. The right stick is RX and RY.
                lt = MotionEvent.AXIS_Z; rt = MotionEvent.AXIS_RZ; rx = MotionEvent.AXIS_RX; ry = MotionEvent.AXIS_RY
            } else {
                // Older DualShock 4 layout: right stick on Z and RZ, triggers on RX and RY.
                lt = MotionEvent.AXIS_RX; rt = MotionEvent.AXIS_RY
            }
        } else if (!zr) { rx = MotionEvent.AXIS_RX; ry = MotionEvent.AXIS_RY }
        ltAxis = lt; rtAxis = rt; rxAxis = rx; ryAxis = ry
    }

    /** Applies a key event. False when the key is not a controller button, so the caller leaves it alone. */
    fun key(code: Int, down: Boolean): Boolean {
        if (code !in HANDLED) return false
        if (down) keys.add(code) else keys.remove(code)
        return true
    }

    /** Reads every axis this pad reports. */
    fun axes(read: (Int) -> Float) {
        for (a in AXES) if (a in ranges) values[a] = read(a)
    }

    fun release() { keys.clear(); values = HashMap() }

    fun state(): PadState {
        var btn = 0
        for (code in keys) BUTTONS[code]?.let { btn = btn or it.mask }
        val hx = values[MotionEvent.AXIS_HAT_X] ?: 0f
        val hy = values[MotionEvent.AXIS_HAT_Y] ?: 0f
        if (hy < -HAT || KeyEvent.KEYCODE_DPAD_UP in keys) btn = btn or PadButton.UP.mask
        if (hy > HAT || KeyEvent.KEYCODE_DPAD_DOWN in keys) btn = btn or PadButton.DOWN.mask
        if (hx < -HAT || KeyEvent.KEYCODE_DPAD_LEFT in keys) btn = btn or PadButton.LEFT.mask
        if (hx > HAT || KeyEvent.KEYCODE_DPAD_RIGHT in keys) btn = btn or PadButton.RIGHT.mask
        val (lx, ly) = deadZone(values[MotionEvent.AXIS_X] ?: 0f, values[MotionEvent.AXIS_Y] ?: 0f)
        val (rx, ry) = deadZone(values[rxAxis] ?: 0f, values[ryAxis] ?: 0f)
        val lt = max(trigger(ltAxis), if (KeyEvent.KEYCODE_BUTTON_L2 in keys) 255 else 0)
        val rt = max(trigger(rtAxis), if (KeyEvent.KEYCODE_BUTTON_R2 in keys) 255 else 0)
        return PadState(btn, lt, rt).stick(true, lx, ly).stick(false, rx, ry)
    }

    private fun trigger(axis: Int?): Int {
        if (axis == null) return 0
        val v = values[axis] ?: return 0
        val (lo, hi) = ranges[axis] ?: return 0
        if (hi <= lo) return 0
        val f = ((v - lo) / (hi - lo)).coerceIn(0f, 1f)
        return if (f < TRIGGER_ZONE) 0 else (f * 255f).toInt()
    }

    companion object {
        const val DEAD_ZONE = 0.08f
        const val TRIGGER_ZONE = 0.04f
        private const val HAT = 0.5f
        private val BUTTONS = mapOf(
            KeyEvent.KEYCODE_BUTTON_A to PadButton.A, KeyEvent.KEYCODE_BUTTON_B to PadButton.B,
            KeyEvent.KEYCODE_BUTTON_X to PadButton.X, KeyEvent.KEYCODE_BUTTON_Y to PadButton.Y,
            KeyEvent.KEYCODE_BUTTON_L1 to PadButton.LB, KeyEvent.KEYCODE_BUTTON_R1 to PadButton.RB,
            KeyEvent.KEYCODE_BUTTON_THUMBL to PadButton.LS, KeyEvent.KEYCODE_BUTTON_THUMBR to PadButton.RS,
            KeyEvent.KEYCODE_BUTTON_START to PadButton.START, KeyEvent.KEYCODE_BUTTON_SELECT to PadButton.BACK,
            KeyEvent.KEYCODE_BUTTON_MODE to PadButton.GUIDE,
        )
        /** Every key code this class consumes. KEYCODE_BACK is not here, so the system Back still works. */
        val HANDLED: Set<Int> = BUTTONS.keys + setOf(
            KeyEvent.KEYCODE_BUTTON_L2, KeyEvent.KEYCODE_BUTTON_R2,
            KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT,
        )
        val AXES = intArrayOf(MotionEvent.AXIS_X, MotionEvent.AXIS_Y, MotionEvent.AXIS_Z, MotionEvent.AXIS_RZ,
            MotionEvent.AXIS_RX, MotionEvent.AXIS_RY, MotionEvent.AXIS_HAT_X, MotionEvent.AXIS_HAT_Y,
            MotionEvent.AXIS_LTRIGGER, MotionEvent.AXIS_RTRIGGER, MotionEvent.AXIS_BRAKE, MotionEvent.AXIS_GAS)

        /** Round dead zone with the rest of the travel stretched to full range, so small drift is silent. */
        fun deadZone(x: Float, y: Float): Pair<Float, Float> {
            val m = hypot(x, y)
            if (m < DEAD_ZONE) return 0f to 0f
            val scale = (m - DEAD_ZONE) / (1f - DEAD_ZONE) / m // not clamped: a square gate still reaches the corners
            return x * scale to y * scale
        }
    }
}

/**
 * Controller ids 0 to 3 in plug-in order. A controller that comes back (same [descriptor]) gets the id it
 * had, unless another controller holds it now.
 */
class PadIds(private val slots: Int = 4) {
    private val live = HashMap<Int, Int>()          // device -> id
    private val liveDescriptor = HashMap<Int, String>()
    private val remembered = LinkedHashMap<String, Int>() // descriptor -> id

    fun idOf(device: Int): Int? = live[device]

    /** The id for a newly plugged controller, or null when all ids are taken. */
    fun attach(device: Int, descriptor: String): Int? {
        live[device]?.let { return it }
        val taken = live.values.toSet()
        // a second pad with the same descriptor (two identical pads) is a new controller
        val duplicate = liveDescriptor.values.contains(descriptor)
        val back = if (duplicate) null else remembered[descriptor]?.takeIf { it !in taken }
        val reservedByOthers = remembered.filterKeys { it != descriptor }.values.toSet()
        val id = back
            ?: (0 until slots).firstOrNull { it !in taken && it !in reservedByOthers }
            ?: (0 until slots).firstOrNull { it !in taken }
            ?: return null
        remembered.entries.removeIf { it.value == id && it.key != descriptor }
        live[device] = id; liveDescriptor[device] = descriptor
        if (!duplicate) remembered[descriptor] = id
        return id
    }

    /** Frees the id of an unplugged controller and returns it. */
    fun detach(device: Int): Int? {
        liveDescriptor.remove(device)
        return live.remove(device)
    }

    fun devices(): Set<Int> = live.keys.toSet()
    fun ids(): Collection<Int> = live.values.toList()
    fun deviceOf(id: Int): Int? = live.entries.firstOrNull { it.value == id }?.key
}

/**
 * Decides when a `pad` message goes out: at once on a change (at most one per [minMs]; the newest state
 * wins), and again after [idleMs] of silence so the host keeps the controller alive.
 */
class PadFeed(private val minMs: Long = 8, private val idleMs: Long = 1000) {
    private class Slot(var last: PadState? = null, var at: Long = 0, var pending: PadState? = null)
    private val slots = HashMap<Int, Slot>()

    fun offer(id: Int, s: PadState, now: Long): Msg.Pad? {
        val slot = slots.getOrPut(id) { Slot() }
        if (s == slot.last) { slot.pending = null; return null }
        if (slot.last == null || now - slot.at >= minMs) return emit(id, slot, s, now)
        slot.pending = s
        return null
    }

    /** Call often. Sends held-back changes and the once-a-second repeat. */
    fun tick(now: Long): List<Msg.Pad> {
        val out = ArrayList<Msg.Pad>()
        for ((id, slot) in slots) {
            val p = slot.pending
            if (p != null && now - slot.at >= minMs) out.add(emit(id, slot, p, now))
            else if (p == null && slot.last != null && now - slot.at >= idleMs) out.add(emit(id, slot, slot.last!!, now))
        }
        return out
    }

    fun drop(id: Int) { slots.remove(id) }

    private fun emit(id: Int, slot: Slot, s: PadState, now: Long): Msg.Pad {
        slot.last = s; slot.at = now; slot.pending = null
        return s.toMsg(id)
    }
}

object Rumble {
    /** Motor strengths (1 to 255) for a device with [motors] motors. Two motors get low then high; one gets the larger. */
    fun amplitudes(lo: Int, hi: Int, motors: Int): IntArray {
        val l = lo.coerceIn(0, 255); val h = hi.coerceIn(0, 255)
        return if (motors >= 2) intArrayOf(l, h) + IntArray(motors - 2) else if (motors == 1) intArrayOf(max(l, h)) else IntArray(0)
    }
}
