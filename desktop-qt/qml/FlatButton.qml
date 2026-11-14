import QtQuick
import QtQuick.Controls.Basic

// The one button shape used across the app. `primary` is the mint action.
AbstractButton {
    id: button
    property bool primary: false
    implicitHeight: Theme.controlHeight
    implicitWidth: Math.max(72, label.implicitWidth + 28)
    opacity: enabled ? 1 : 0.45

    contentItem: Text {
        id: label
        text: button.text
        color: button.primary ? Theme.background : Theme.text
        font.pixelSize: Theme.textSize
        font.weight: Font.DemiBold
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    background: Rectangle {
        radius: Theme.radius
        color: button.primary ? (button.down ? Qt.darker(Theme.mint, 1.15) : Theme.mint)
                              : (button.down || button.hovered ? Theme.border : Theme.surfaceRaised)
        border.width: 1
        border.color: button.visualFocus ? Theme.text : (button.primary ? Theme.mint : Theme.border)
    }
}
