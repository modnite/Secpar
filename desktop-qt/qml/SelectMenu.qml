import QtQuick
import QtQuick.Controls.Basic

// The app's dropdown: a flat list under the row that holds the value. The
// current value is marked with a check. Space or a click opens it, the arrow keys
// move, Enter or Space picks, Escape closes.
Popup {
    id: menu

    // [[value, label], ...] in the order the service or the stream accepts them.
    property var options: []
    property var current: undefined
    // The page hands back what was picked; the row that opened the menu is told
    // which value it shows.
    property string key
    property Item anchor: null
    // The page rectangle the menu must stay inside.
    property Item bound: null
    // The item that gets the keyboard focus back when the menu closes.
    property Item restore: null
    property int rowHeight: 32
    property int highlighted: 0

    signal picked(string key, var value)

    parent: null
    z: 100
    padding: 1
    modal: false
    dim: false
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    // The basic style dims the window under a popup. This menu is part of the row.
    Overlay.modeless: Rectangle { color: "transparent" }

    FontMetrics {
        id: metrics
        font.pixelSize: Theme.textSize
    }

    function openFor(anchorItem, keyName, list, value, inside, focusBack) {
        menu.anchor = anchorItem
        menu.key = keyName
        menu.options = list
        menu.current = value
        menu.bound = inside || null
        menu.restore = focusBack || null
        // Positioned inside the page the row belongs to, so its coordinates and
        // clamping are the page's own.
        menu.parent = inside || null
        let at = 0
        for (let i = 0; i < list.length; ++i) {
            if (list[i][0] === value)
                at = i
        }
        menu.highlighted = at
        menu.open()
    }

    // The menu sits under the value, right edges aligned, and turns up when the
    // page has no room below it.
    function place() {
        if (!anchor || !bound)
            return
        let widest = 0
        for (let i = 0; i < options.length; ++i)
            widest = Math.max(widest, metrics.advanceWidth(options[i][1]))
        width = Math.max(anchor.width, widest + 46)
        height = options.length * rowHeight + 2
        const at = anchor.mapToItem(bound, 0, 0)
        x = Math.max(6, Math.min(at.x + anchor.width - width, bound.width - width - 6))
        let top = at.y + anchor.height + 3
        if (top + height > bound.height - 6)
            top = at.y - height - 3
        y = Math.max(6, Math.min(top, bound.height - height - 6))
    }

    function choose(value) {
        const keyName = menu.key
        menu.close()
        menu.picked(keyName, value)
    }

    function move(step) {
        if (options.length === 0)
            return
        highlighted = Math.max(0, Math.min(options.length - 1, highlighted + step))
    }

    onAboutToShow: place()
    onOpened: list.forceActiveFocus()
    onClosed: {
        if (restore && restore.visible)
            restore.forceActiveFocus()
    }

    background: Rectangle {
        color: Theme.surface
        radius: 7
        border.width: 1
        border.color: Theme.border
    }

    contentItem: ListView {
        id: list
        implicitHeight: menu.options.length * menu.rowHeight
        clip: true
        interactive: false
        keyNavigationEnabled: false
        boundsBehavior: Flickable.StopAtBounds
        model: menu.options

        Keys.onPressed: function (event) {
            if (event.key === Qt.Key_Down) {
                menu.move(1)
                event.accepted = true
            } else if (event.key === Qt.Key_Up) {
                menu.move(-1)
                event.accepted = true
            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                if (menu.options.length > 0)
                    menu.choose(menu.options[menu.highlighted][0])
                event.accepted = true
            } else if (event.key === Qt.Key_Escape) {
                menu.close()
                event.accepted = true
            }
        }

        delegate: Rectangle {
            id: option
            required property var modelData
            required property int index
            width: ListView.view.width
            height: menu.rowHeight
            color: option.index === menu.highlighted ? Theme.surfaceRaised : "transparent"

            Row {
                anchors.left: parent.left
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6
                Glyph {
                    anchors.verticalCenter: parent.verticalCenter
                    icon: "check"
                    size: 14
                    color: Theme.mint
                    // Hidden rather than skipped, so every label starts at the same edge.
                    opacity: option.modelData[0] === menu.current ? 1 : 0
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: option.modelData[1]
                    color: option.modelData[0] === menu.current ? Theme.text : Theme.textDim
                    font.pixelSize: Theme.textSize
                }
            }

            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onEntered: menu.highlighted = option.index
                onClicked: menu.choose(option.modelData[0])
            }
        }
    }
}
