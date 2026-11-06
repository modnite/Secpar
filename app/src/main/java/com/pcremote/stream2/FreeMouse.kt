package com.pcremote.stream2

import android.view.MotionEvent
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.pointer.*
import com.pcremote.stream.ZoomPan

/** Part of the video hit target, never an Activity-wide interceptor. Android
 * widgets above it and windows outside it retain normal mouse behaviour. */
internal fun Modifier.freeMouse(
    bounds: Rect,
    zoom: ZoomPan,
    input: DesktopInput,
    enabled: () -> Boolean,
    inside: (Boolean) -> Unit,
) = pointerInput(bounds, zoom, input) {
    try {
        awaitPointerEventScope {
            while (true) {
                val event=awaitPointerEvent()
                val mouse=event.changes.firstOrNull {it.type==PointerType.Mouse} ?: continue
                val point=Offset(zoom.toStreamX(mouse.position.x),zoom.toStreamY(mouse.position.y))
                if (event.type==PointerEventType.Exit || !enabled() || bounds.isEmpty || !bounds.contains(point)) {
                    inside(false);input.releaseMouse()
                    continue
                }
                inside(true)
                val b=event.buttons
                val buttons=(if(b.isPrimaryPressed)MotionEvent.BUTTON_PRIMARY else 0) or
                    (if(b.isSecondaryPressed)MotionEvent.BUTTON_SECONDARY else 0) or
                    (if(b.isTertiaryPressed)MotionEvent.BUTTON_TERTIARY else 0) or
                    (if(b.isBackPressed)MotionEvent.BUTTON_BACK else 0) or
                    (if(b.isForwardPressed)MotionEvent.BUTTON_FORWARD else 0)
                input.absoluteMouse((point.x-bounds.left)/bounds.width,(point.y-bounds.top)/bounds.height,
                    buttons,mouse.scrollDelta.x,mouse.scrollDelta.y)
                event.changes.forEach {it.consume()}
            }
        }
    } finally { inside(false);input.releaseMouse() }
}
