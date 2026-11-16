import QtQuick
import QtQuick.Controls.Basic

// The one text field shape used across the app, the same height as FlatButton.
TextField {
    id: field
    implicitHeight: Theme.controlHeight
    leftPadding: 10
    rightPadding: 10
    color: Theme.text
    placeholderTextColor: Theme.textDim
    selectionColor: Theme.mint
    selectedTextColor: Theme.background
    font.pixelSize: Theme.textSize
    verticalAlignment: TextInput.AlignVCenter
    background: Rectangle {
        radius: Theme.radius
        color: Theme.surface
        border.width: 1
        border.color: field.activeFocus ? Theme.text : Theme.border
    }
}
