package com.pcremote.modes

import android.content.Context
import android.hardware.input.InputManager
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.SystemClock
import android.os.VibratorManager
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import com.pcremote.MainActivity
import com.pcremote.agent.Msg
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow

/** Android events from physical controllers into [PhysicalPads]. */
object PadInput {
    private fun fromPad(source: Int) = source and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD

    /** True when the key was a controller button and was taken. */
    fun key(pads: PhysicalPads, e: KeyEvent): Boolean {
        if (!fromPad(e.source) || !PhysicalPads.isPadKey(e.keyCode)) return false
        return when (e.action) {
            KeyEvent.ACTION_DOWN -> if (e.repeatCount > 0) true else pads.key(e.deviceId, e.keyCode, true)
            KeyEvent.ACTION_UP -> pads.key(e.deviceId, e.keyCode, false)
            else -> false
        }
    }

    fun motion(pads: PhysicalPads, e: MotionEvent): Boolean {
        if (e.source and InputDevice.SOURCE_JOYSTICK != InputDevice.SOURCE_JOYSTICK || e.actionMasked != MotionEvent.ACTION_MOVE) return false
        val d = e.device ?: return false
        fun has(axis: Int) = d.getMotionRange(axis, e.source) != null
        fun v(axis: Int) = if (has(axis)) e.getAxisValue(axis) else 0f
        val rightOnRxRy = has(MotionEvent.AXIS_RX) && has(MotionEvent.AXIS_RY)
        val (rx, ry) = if (rightOnRxRy) v(MotionEvent.AXIS_RX) to v(MotionEvent.AXIS_RY) else v(MotionEvent.AXIS_Z) to v(MotionEvent.AXIS_RZ)
        val (lt, rt) = when {
            has(MotionEvent.AXIS_LTRIGGER) || has(MotionEvent.AXIS_RTRIGGER) -> v(MotionEvent.AXIS_LTRIGGER) to v(MotionEvent.AXIS_RTRIGGER)
            has(MotionEvent.AXIS_BRAKE) || has(MotionEvent.AXIS_GAS) -> v(MotionEvent.AXIS_BRAKE) to v(MotionEvent.AXIS_GAS)
            rightOnRxRy -> v(MotionEvent.AXIS_Z).coerceAtLeast(0f) to v(MotionEvent.AXIS_RZ).coerceAtLeast(0f)
            else -> 0f to 0f
        }
        fun hat(x: Float) = if (x < -0.5f) -1 else if (x > 0.5f) 1 else 0
        return pads.axes(
            e.deviceId,
            PadAxes(v(MotionEvent.AXIS_X), v(MotionEvent.AXIS_Y), rx, ry, lt, rt, hat(v(MotionEvent.AXIS_HAT_X)), hat(v(MotionEvent.AXIS_HAT_Y))),
        )
    }

    /** Rumble from the host to the controller that owns pad [id]. */
    fun rumble(pads: PhysicalPads, id: Int, lo: Int, hi: Int) {
        val device = pads.deviceOf(id)?.let(InputDevice::getDevice) ?: return
        val vibrator: Vibrator? = if (Build.VERSION.SDK_INT >= 31) device.vibratorManager.defaultVibrator
        else @Suppress("DEPRECATION") device.vibrator
        if (vibrator?.hasVibrator() != true) return
        val amplitude = maxOf(lo, hi)
        if (amplitude == 0) vibrator.cancel() else vibrator.vibrate(VibrationEffect.createOneShot(5_000, amplitude.coerceIn(1, 255)))
    }
}

/**
 * Forwards physical controllers while this screen is up. [enabled] is true when the host offers
 * `pad`. [firstId] is the lowest pad id they may use (the on-screen pad keeps 0).
 */
@Composable
fun PhysicalPadsEffect(send: (Msg) -> Unit, incoming: Flow<Msg>?, enabled: Boolean, firstId: Int = 0) {
    val ctx = LocalContext.current
    val activity = ctx as? MainActivity
    val currentSend by rememberUpdatedState(send)
    val pads = remember(enabled, firstId) {
        if (enabled) PhysicalPads({ m -> currentSend(m) }, firstId) else null
    }
    DisposableEffect(pads) {
        activity?.physicalPads = pads
        val manager = ctx.getSystemService(Context.INPUT_SERVICE) as InputManager
        val listener = object : InputManager.InputDeviceListener {
            override fun onInputDeviceAdded(deviceId: Int) {}
            override fun onInputDeviceChanged(deviceId: Int) {}
            override fun onInputDeviceRemoved(deviceId: Int) { pads?.removed(deviceId) }
        }
        if (pads != null) manager.registerInputDeviceListener(listener, null)
        onDispose {
            if (pads != null) manager.unregisterInputDeviceListener(listener)
            pads?.releaseAll()
            if (activity?.physicalPads === pads) activity?.physicalPads = null
        }
    }
    LaunchedEffect(pads) {
        if (pads != null) while (true) { pads.flush(SystemClock.uptimeMillis()); delay(8) }
    }
    LaunchedEffect(pads, incoming) {
        if (pads != null) incoming?.collect { m -> if (m is Msg.Rumble) PadInput.rumble(pads, m.id, m.lo, m.hi) }
    }
}
