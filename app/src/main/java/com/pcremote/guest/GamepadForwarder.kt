package com.pcremote.guest

import android.content.Context
import android.hardware.input.InputManager
import android.os.Build
import android.os.CombinedVibration
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.os.VibrationEffect
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import com.pcremote.agent.Msg
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Sends the USB and Bluetooth controllers attached to this device to the friend's computer as `pad`
 * messages. Everything runs on the main thread: input events, hot plug callbacks and the timer.
 */
class GamepadForwarder(ctx: Context, private val send: (Msg) -> Boolean, private val scope: CoroutineScope) {
    private val manager = ctx.getSystemService(InputManager::class.java)
    private val ids = PadIds()
    private val feed = PadFeed()
    private val mappers = HashMap<Int, PadMapper>()
    private class Rumbling(val lo: Int, val hi: Int, val received: Long, var played: Long)
    private val rumbling = HashMap<Int, Rumbling>()
    private var ticker: Job? = null
    private val listener = object : InputManager.InputDeviceListener {
        override fun onInputDeviceAdded(deviceId: Int) { add(deviceId) }
        override fun onInputDeviceRemoved(deviceId: Int) { remove(deviceId) }
        override fun onInputDeviceChanged(deviceId: Int) {}
    }

    fun start() {
        manager.registerInputDeviceListener(listener, Handler(Looper.getMainLooper()))
        for (id in manager.inputDeviceIds) add(id)
        ticker = scope.launch {
            while (isActive) {
                delay(10)
                val now = SystemClock.uptimeMillis()
                feed.tick(now).forEach { send(it) }
                rearmRumble(now)
            }
        }
    }

    fun stop() {
        ticker?.cancel(); ticker = null
        manager.unregisterInputDeviceListener(listener)
        for (device in ids.devices().toList()) remove(device)
    }

    /** Called for every key event; true when it was a controller button and is handled here. */
    fun onKey(e: KeyEvent): Boolean {
        if (e.keyCode !in PadMapper.HANDLED || !isPad(e.device)) return false
        val mapper = mapperFor(e.deviceId) ?: return false
        if (e.action == KeyEvent.ACTION_DOWN || e.action == KeyEvent.ACTION_UP) {
            mapper.key(e.keyCode, e.action == KeyEvent.ACTION_DOWN)
            submit(e.deviceId, mapper)
        }
        return true
    }

    /** Called for every generic motion event; true when it came from a controller stick, trigger or hat. */
    fun onMotion(e: MotionEvent): Boolean {
        if (!e.isFromSource(InputDevice.SOURCE_JOYSTICK) || e.actionMasked != MotionEvent.ACTION_MOVE) return false
        val mapper = mapperFor(e.deviceId) ?: return false
        mapper.axes(e::getAxisValue)
        submit(e.deviceId, mapper)
        return true
    }

    /** The host's `rumble` for one of our controller ids. */
    fun rumble(m: Msg.Rumble) {
        val device = ids.deviceOf(m.id) ?: return
        val now = SystemClock.uptimeMillis()
        if (m.lo <= 0 && m.hi <= 0) rumbling.remove(m.id) else rumbling[m.id] = Rumbling(m.lo, m.hi, now, now)
        play(device, m.lo, m.hi)
    }

    private fun rearmRumble(now: Long) {
        val it = rumbling.entries.iterator()
        while (it.hasNext()) {
            val (id, r) = it.next()
            if (now - r.received > RUMBLE_LIMIT_MS) { it.remove(); ids.deviceOf(id)?.let { d -> play(d, 0, 0) }; continue }
            if (now - r.played > RUMBLE_MS - 1000) { r.played = now; ids.deviceOf(id)?.let { d -> play(d, r.lo, r.hi) } }
        }
    }

    private fun play(deviceId: Int, lo: Int, hi: Int) {
        val device = InputDevice.getDevice(deviceId) ?: return
        try {
            if (Build.VERSION.SDK_INT >= 31) {
                val vm = device.vibratorManager
                val motors = vm.vibratorIds
                val amps = Rumble.amplitudes(lo, hi, motors.size)
                if (amps.none { it > 0 }) { vm.cancel(); return }
                val combined = CombinedVibration.startParallel()
                motors.forEachIndexed { i, motor ->
                    if (amps[i] > 0) combined.addVibrator(motor, VibrationEffect.createOneShot(RUMBLE_MS, amps[i]))
                }
                vm.vibrate(combined.combine())
            } else {
                @Suppress("DEPRECATION") val vibrator = device.vibrator
                val amp = Rumble.amplitudes(lo, hi, 1).firstOrNull() ?: 0
                if (amp == 0) vibrator.cancel()
                else if (vibrator.hasVibrator()) vibrator.vibrate(VibrationEffect.createOneShot(RUMBLE_MS, amp))
            }
        } catch (_: RuntimeException) {
            // a pad that cannot rumble, or that was unplugged a moment ago
        }
    }

    private fun mapperFor(deviceId: Int): PadMapper? = mappers[deviceId] ?: add(deviceId)

    private fun add(deviceId: Int): PadMapper? {
        mappers[deviceId]?.let { return it }
        val device = InputDevice.getDevice(deviceId)
        if (!isPad(device)) return null
        device!!
        val id = ids.attach(deviceId, device.descriptor.orEmpty()) ?: return null
        val ranges = HashMap<Int, Pair<Float, Float>>()
        for (axis in PadMapper.AXES) device.getMotionRange(axis, InputDevice.SOURCE_JOYSTICK)?.let { ranges[axis] = it.min to it.max }
        val mapper = PadMapper(ranges)
        mappers[deviceId] = mapper
        // the host creates the controller on its first state
        feed.offer(id, mapper.state(), SystemClock.uptimeMillis())?.let { send(it) }
        return mapper
    }

    private fun remove(deviceId: Int) {
        mappers.remove(deviceId)
        val id = ids.detach(deviceId) ?: return
        feed.drop(id)
        rumbling.remove(id)
        send(Msg.PadOff(id))
    }

    private fun submit(deviceId: Int, mapper: PadMapper) {
        val id = ids.idOf(deviceId) ?: return
        feed.offer(id, mapper.state(), SystemClock.uptimeMillis())?.let { send(it) }
    }

    companion object {
        private const val RUMBLE_MS = 5_000L
        private const val RUMBLE_LIMIT_MS = 30_000L

        /** A real game controller: not a keyboard, a touch screen or a virtual device. */
        fun isPad(d: InputDevice?): Boolean {
            if (d == null || d.isVirtual) return false
            val s = d.sources
            return (s and InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD ||
                ((s and InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK && d.getMotionRange(MotionEvent.AXIS_X) != null)
        }
    }
}
