import QtQuick
import QtQuick.Controls.Basic

AbstractButton {
    id: tab
    property bool current: false
    implicitHeight: 48
    implicitWidth: label.implicitWidth + 24
    focusPolicy: Qt.TabFocus

    contentItem: Text {
        id: label
        text: tab.text
        color: tab.current || tab.hovered ? Theme.text : Theme.textDim
        font.pixelSize: Theme.textSize
        font.weight: tab.current ? Font.DemiBold : Font.Normal
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
    background: Item {
        Rectangle {
            anchors.bottom: parent.bottom
            anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width - 24
            height: 2
            color: Theme.mint
            visible: tab.current
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: 6
            radius: Theme.radius
            color: "transparent"
            border.color: Theme.mint
            border.width: tab.visualFocus ? 1 : 0
        }
    }
}
