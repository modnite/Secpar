package com.pcremote.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import com.pcremote.R
import com.pcremote.stream.StreamMenuRow
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MenuColors
import com.pcremote.ui.theme.MenuTokens

/** One row of an [AnchoredMenu]. */
class MenuItem(val icon: Int, val label: String, val destructive: Boolean = false, val keepOpen: Boolean = false, val onClick: () -> Unit)

/** Drops the popup under the anchor, right edges aligned, kept inside the window. */
private class BelowEnd : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize): IntOffset {
        val x = (anchorBounds.right - popupContentSize.width).coerceIn(0, (windowSize.width - popupContentSize.width).coerceAtLeast(0))
        val y = anchorBounds.bottom.coerceAtMost((windowSize.height - popupContentSize.height).coerceAtLeast(0))
        return IntOffset(x, y)
    }
}

/**
 * The "…" button and its menu. The panel uses the stream menu numbers: rows 34dp, Lucide icons 20dp,
 * 6dp padding, radius 5.
 */
@Composable
fun AnchoredMenu(label: String, items: List<MenuItem>, modifier: Modifier = Modifier) {
    val c = LocalMidnight.current
    var open by remember { mutableStateOf(false) }
    Box(modifier) {
        IconButton({ open = true }, Modifier.size(48.dp)) {
            Icon(painterResource(R.drawable.lucide_ellipsis), label, tint = c.textDim, modifier = Modifier.size(20.dp))
        }
        if (open) Popup(BelowEnd(), { open = false }, PopupProperties(focusable = true)) {
            Column(Modifier.width(MenuTokens.PANEL_WIDTH.dp).clip(RoundedCornerShape(MenuTokens.PANEL_RADIUS.dp))
                .background(MenuColors.fill).padding(MenuTokens.PANEL_PADDING.dp)) {
                items.forEach { item ->
                    StreamMenuRow(item.icon, item.label, destructive = item.destructive) {
                        if (!item.keepOpen) open = false
                        item.onClick()
                    }
                }
            }
        }
    }
}
