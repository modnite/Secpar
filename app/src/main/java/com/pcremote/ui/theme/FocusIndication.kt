package com.pcremote.ui.theme

import androidx.compose.foundation.IndicationNodeFactory
import androidx.compose.foundation.interaction.FocusInteraction
import androidx.compose.foundation.interaction.InteractionSource
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.node.DelegatableNode
import androidx.compose.ui.node.DelegatingNode
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.invalidateDraw
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

/**
 * The app's click feedback: the touch ripple, plus a 2dp accent outline while the element has
 * focus. A TV remote or keyboard moves focus; without the outline the default focus tint was
 * barely visible on Android TV.
 */
class FocusIndication(private val ring: Color, private val ripple: IndicationNodeFactory) : IndicationNodeFactory {
    override fun create(interactionSource: InteractionSource): DelegatableNode = Node(interactionSource, ring, ripple.create(interactionSource))
    override fun equals(other: Any?) = other is FocusIndication && other.ring == ring && other.ripple == ripple
    override fun hashCode() = 31 * ring.hashCode() + ripple.hashCode()

    private class Node(private val source: InteractionSource, private val ring: Color, ripple: DelegatableNode) : DelegatingNode(), DrawModifierNode {
        private var focused = false

        init { delegate(ripple) }

        override fun onAttach() {
            coroutineScope.launch {
                var count = 0
                source.interactions.collect { i ->
                    when (i) {
                        is FocusInteraction.Focus -> count++
                        is FocusInteraction.Unfocus -> count--
                    }
                    if ((count > 0) != focused) { focused = count > 0; invalidateDraw() }
                }
            }
        }

        override fun ContentDrawScope.draw() {
            drawContent()
            if (!focused) return
            val w = 2.dp.toPx()
            drawRoundRect(ring, topLeft = Offset(w / 2, w / 2), size = Size(size.width - w, size.height - w),
                cornerRadius = CornerRadius(5.dp.toPx()), style = Stroke(w))
        }
    }
}
