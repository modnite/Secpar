import QtQuick

// The current value of a choice, in the flat box that opens its list. The page
// decides what a click does; the row passes its keyboard focus state in. Where
// the row itself is not focusable, `tabStop` makes the value reachable by Tab
// and Enter or Space opens it.
Rectangle {
    id: control
    property string text
    property bool rowFocus: false
    property bool tabStop: false
    signal clicked()

    implicitHeight: 26
    implicitWidth: Math.max(132, label.implicitWidth + 38)
    radius: Theme.radius
    color: area.containsMouse ? Theme.surfaceRaised : Theme.surface
    border.width: 1
    border.color: control.rowFocus || control.activeFocus ? Theme.text : Theme.border
    activeFocusOnTab: control.tabStop

    Keys.onPressed: function (event) {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter
                || event.key === Qt.Key_Space || event.key === Qt.Key_Down) {
            control.clicked()
            event.accepted = true
        }
    }

    Text {
        id: label
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.right: chevron.left
        anchors.rightMargin: 6
        text: control.text
        color: Theme.text
        font.pixelSize: Theme.textSize
        elide: Text.ElideRight
    }
    // At the right edge, so the arrows line up down the page.
    Glyph {
        id: chevron
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: 8
        icon: "chevron-down"
        size: 14
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: control.clicked()
    }
}
