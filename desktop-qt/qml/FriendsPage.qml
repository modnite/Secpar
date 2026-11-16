import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

// Friends, one row each: who they are and whether they join without being asked.
// Tapping an accepted friend opens the two permissions that decide what they may
// do. Requests waiting for an answer come first, then a row for adding someone.
Item {
    id: page
    // Which friend's permissions are open. A handle names one row.
    property string expandedHandle: ""

    // Fictional people for layout review only (--sample).
    property var samplePeople: [
        { kind: "request", id: "s4", name: "Robin", handle: "robin", state: "pending",
          connect: "ask", keyboard: false, mouse: false },
        { kind: "friend", id: "s1", name: "Mika", handle: "mika", state: "accepted",
          connect: "always", input: "full", keyboard: true, mouse: true },
        { kind: "friend", id: "s2", name: "Jo", handle: "jo", state: "accepted",
          connect: "ask", input: "view", keyboard: false, mouse: false },
        { kind: "friend", id: "s3", name: "Sam", handle: "sam", state: "accepted",
          connect: "ask", input: "full", keyboard: true, mouse: true }
    ]
    readonly property var people: {
        if (sampleData)
            return page.samplePeople
        const rows = []
        for (const request of backend.friendRequests)
            rows.push(Object.assign({ kind: "request" }, request))
        for (const friend of backend.friends)
            rows.push(Object.assign({ kind: "friend" }, friend))
        return rows
    }
    function title(person) {
        return person.name && person.name.length > 0 ? person.name : "@" + person.handle
    }
    // What the two keyboard and mouse permissions add up to, said plainly.
    function control(person) {
        if (person.keyboard && person.mouse)
            return "Keyboard and mouse"
        if (person.keyboard)
            return "Keyboard only"
        if (person.mouse)
            return "Mouse only"
        return "View only"
    }
    function controlValue(person) {
        if (person.keyboard && person.mouse)
            return "full"
        if (person.keyboard)
            return "keyboard"
        if (person.mouse)
            return "mouse"
        return "view"
    }
    function connectLabel(person) {
        return person.connect === "always" ? "Always" : "Ask each time"
    }
    function summary(person) {
        return "@" + person.handle + " · " + page.control(person)
    }
    function accepted(person) {
        return person.kind === "friend" && person.state === "accepted"
    }
    // The first accepted friend, used by the review capture to show one editor open.
    readonly property string firstAccepted: {
        for (const person of page.people) {
            if (page.accepted(person))
                return person.handle
        }
        return ""
    }
    Component.onCompleted: {
        if (sampleData && sampleOpen)
            page.expandedHandle = page.firstAccepted
    }
    function expanded(person) {
        return page.accepted(person) && page.expandedHandle === person.handle
    }
    function toggle(person) {
        if (!page.accepted(person))
            return
        page.expandedHandle = page.expanded(person) ? "" : person.handle
    }
    function sendRequest() {
        const handle = field.text.trim()
        if (handle.length === 0 || sampleData)
            return
        backend.addFriend(handle)
    }
    // The sample changes its own copy; the real page asks the account service and
    // takes the answer from there.
    function changeSample(id, change) {
        page.samplePeople = page.samplePeople.map(person =>
            person.id === id ? Object.assign({}, person, change) : person)
    }
    function setControl(id, value) {
        const keyboard = value === "full" || value === "keyboard"
        const mouse = value === "full" || value === "mouse"
        if (sampleData)
            page.changeSample(id, { keyboard: keyboard, mouse: mouse, input: keyboard && mouse ? "full" : "view" })
        else
            backend.saveFriendSettings(id, { "keyboard": keyboard, "mouse": mouse })
    }

    Connections {
        target: backend
        function onFriendRequestSent(sent) {
            if (sent)
                field.text = ""
        }
    }

    RowLayout {
        id: addRow
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Theme.margin
        spacing: Theme.gap
        FlatField {
            id: field
            Layout.preferredWidth: 210
            Layout.maximumWidth: 240
            placeholderText: "@account name"
            onAccepted: page.sendRequest()
        }
        FlatButton {
            text: "Send request"
            primary: true
            enabled: field.text.trim().length > 0 && (sampleData || !backend.busy)
            onClicked: page.sendRequest()
        }
        Item { Layout.fillWidth: true }
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: addRow.bottom
        anchors.bottom: parent.bottom
        anchors.leftMargin: Theme.margin
        anchors.rightMargin: Theme.margin
        anchors.topMargin: Theme.gap
        anchors.bottomMargin: notice.visible ? notice.height + 8 : Theme.margin
        visible: page.people.length > 0
        clip: true
        spacing: 0
        model: page.people
        delegate: Rectangle {
            id: personRow
            required property var modelData
            required property int index
            readonly property bool expandable: page.accepted(modelData)
            readonly property bool open: page.expanded(modelData)
            width: ListView.view.width
            height: body.implicitHeight
            color: "transparent"
            activeFocusOnTab: personRow.expandable
            Keys.onPressed: function (event) {
                if (!personRow.expandable)
                    return
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                    page.toggle(personRow.modelData)
                    event.accepted = true
                }
            }

            ColumnLayout {
                id: body
                width: parent.width
                spacing: 0

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 56
                    color: rowMouse.containsMouse && personRow.expandable ? Theme.surface : "transparent"
                    border.width: personRow.activeFocus ? 1 : 0
                    border.color: Theme.text
                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        enabled: personRow.expandable
                        hoverEnabled: true
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: page.toggle(personRow.modelData)
                    }
                    RowLayout {
                        anchors.fill: parent
                        spacing: Theme.gap
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2
                            Text {
                                Layout.fillWidth: true
                                text: page.title(personRow.modelData)
                                color: Theme.text
                                font.pixelSize: Theme.textSize
                                font.weight: Font.DemiBold
                                elide: Text.ElideRight
                            }
                            Text {
                                Layout.fillWidth: true
                                text: personRow.modelData.kind === "request"
                                      ? "@" + personRow.modelData.handle
                                      : page.summary(personRow.modelData)
                                color: Theme.textDim
                                font.pixelSize: Theme.smallTextSize
                                elide: Text.ElideRight
                            }
                        }
                        Text {
                            visible: personRow.modelData.kind === "request"
                                     || personRow.modelData.state === "pending"
                            text: personRow.modelData.state === "pending" && personRow.modelData.kind !== "request"
                                  ? "Request sent" : "Wants to be your friend"
                            color: Theme.textDim
                            font.pixelSize: Theme.smallTextSize
                        }
                        Text {
                            visible: page.accepted(personRow.modelData)
                            text: page.connectLabel(personRow.modelData)
                            color: personRow.modelData.connect === "always" ? Theme.mint : Theme.textDim
                            font.pixelSize: Theme.smallTextSize
                        }
                        FlatButton {
                            visible: personRow.modelData.kind === "request"
                            text: "Accept"
                            primary: true
                            onClicked: {
                                if (!sampleData)
                                    backend.acceptFriend(personRow.modelData.id)
                            }
                        }
                        FlatButton {
                            visible: personRow.modelData.kind === "request"
                            text: "Decline"
                            onClicked: {
                                if (!sampleData)
                                    backend.removeFriend(personRow.modelData.id)
                            }
                        }
                        FlatButton {
                            visible: personRow.modelData.kind === "friend"
                            text: personRow.modelData.state === "pending" ? "Cancel" : "Remove"
                            onClicked: {
                                if (!sampleData)
                                    backend.removeFriend(personRow.modelData.id)
                            }
                        }
                        Glyph {
                            visible: personRow.expandable
                            icon: personRow.open ? "chevron-down" : "chevron-right"
                            size: 14
                        }
                    }
                }

                // The two permissions, saved as soon as one is chosen.
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.leftMargin: 12
                    Layout.bottomMargin: 8
                    visible: personRow.open
                    spacing: 4
                    RowLayout {
                        Layout.fillWidth: true
                        implicitHeight: 32
                        spacing: Theme.gap
                        Text {
                            Layout.fillWidth: true
                            text: "Allow to connect"
                            color: Theme.text
                            font.pixelSize: Theme.textSize
                        }
                        ValueButton {
                            id: connectControl
                            width: implicitWidth
                            tabStop: true
                            text: page.connectLabel(personRow.modelData)
                            onClicked: connectMenu.openFor(connectControl, personRow.modelData.id,
                                                           [["ask", "Ask each time"], ["always", "Always"]],
                                                           personRow.modelData.connect || "ask", page, connectControl)
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        implicitHeight: 32
                        spacing: Theme.gap
                        Text {
                            Layout.fillWidth: true
                            text: "Control"
                            color: Theme.text
                            font.pixelSize: Theme.textSize
                        }
                        ValueButton {
                            id: controlControl
                            width: implicitWidth
                            tabStop: true
                            text: page.control(personRow.modelData)
                            onClicked: controlMenu.openFor(controlControl, personRow.modelData.id,
                                                           [["view", "View only"], ["keyboard", "Keyboard only"], ["mouse", "Mouse only"], ["full", "Keyboard and mouse"]],
                                                           page.controlValue(personRow.modelData), page, controlControl)
                        }
                    }
                }
            }

            // The editor under the row grows the delegate.
        }
    }

    // What went wrong with the last action, along the bottom edge.
    Rectangle {
        id: notice
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        visible: !sampleData && backend.message.length > 0
        height: noticeText.implicitHeight + 16
        color: Theme.surface
        Rectangle { width: parent.width; height: 1; color: Theme.border }
        Text {
            id: noticeText
            anchors.verticalCenter: parent.verticalCenter
            x: Theme.margin
            width: parent.width - 2 * Theme.margin
            text: backend.message
            color: Theme.orange
            font.pixelSize: Theme.smallTextSize
            wrapMode: Text.WordWrap
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        anchors.verticalCenterOffset: addRow.height / 2
        visible: page.people.length === 0 && backend.message.length === 0
        spacing: 6
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "No friends yet"
            color: Theme.text
            font.pixelSize: Theme.textSize
            font.weight: Font.DemiBold
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "Enter an account name above to send a request."
            color: Theme.textDim
            font.pixelSize: Theme.smallTextSize
        }
    }

    SelectMenu {
        id: connectMenu
        onPicked: function (id, value) {
            if (sampleData)
                page.changeSample(id, { connect: value })
            else
                backend.saveFriendSettings(id, { "connect": value })
        }
    }

    SelectMenu {
        id: controlMenu
        onPicked: function (id, value) { page.setControl(id, value) }
    }
}
