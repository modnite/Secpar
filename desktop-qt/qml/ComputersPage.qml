import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtQuick.Dialogs

// Own and friends' computers in one grid. Two tiles per row at 640 wide, more as
// the window grows.
Item {
    id: page
    // Fictional entries for layout review only (--sample).
    readonly property var sample: [
        { name: "Living room PC", owner: "", online: true, route: "This network" },
        { name: "Workshop laptop", owner: "", online: true, route: "Direct" },
        { name: "Mika's desktop", owner: "Mika", online: true, route: "Relay" },
        { name: "Attic server", owner: "", online: false, route: "" }
    ]
    // The account's other hosting computers.
    readonly property var own: backend.devices.filter(d => d.hosts && !d.this).map(d => (
        { id: d.id, name: d.name, owner: "", online: d.online, route: "",
          status: backend.connectedId === d.id ? (backend.streaming ? "Streaming" : "Connected")
                : backend.connectingId === d.id ? "Connecting" : "" }))
    readonly property var computers: sampleData ? sample : (backend.background ? own : [])
    // One computer is connected, so its tile carries the row for sending a file.
    readonly property bool connectedHere: !sampleData && backend.connectedId.length > 0

    // A still is only asked for while the page can be seen.
    Component.onCompleted: backend.watchPreviews(visible)
    onVisibleChanged: backend.watchPreviews(visible)

    // The file to send goes to the connected computer.
    FileDialog {
        id: picker
        title: "Send a file"
        fileMode: FileDialog.OpenFile
        onAccepted: backend.sendFile(selectedFile)
    }

    ScrollView {
        anchors.fill: parent
        contentWidth: availableWidth
        visible: page.computers.length > 0

        GridLayout {
            id: grid
            readonly property int minimumTile: 220
            // Keep two rows of tiles on screen without scrolling when the window
            // allows it, with room for the row a connected tile adds.
            readonly property int previewLimit: Math.max(72, Math.floor((page.height - 2 * Theme.margin - Theme.gap) / 2) - 58 - (page.connectedHere ? 44 : 0))
            x: Theme.margin
            y: Theme.margin
            width: page.width - 2 * Theme.margin
            columns: Math.max(2, Math.floor((width + Theme.gap) / (minimumTile + Theme.gap)))
            columnSpacing: Theme.gap
            rowSpacing: Theme.gap

            Repeater {
                model: page.computers
                ComputerTile {
                    Layout.fillWidth: false
                    Layout.preferredWidth: (grid.width - (grid.columns - 1) * Theme.gap) / grid.columns
                    Layout.alignment: Qt.AlignTop
                    name: modelData.name
                    owner: modelData.owner
                    online: modelData.online
                    route: modelData.route
                    status: modelData.status || ""
                    connected: !sampleData && backend.connectedId === modelData.id
                    streaming: !sampleData && backend.streaming && backend.connectedId === modelData.id
                    busy: !sampleData && (backend.busy || (backend.connectedId.length > 0 && backend.connectedId !== modelData.id))
                    sendVisible: !sampleData && backend.connectedId === modelData.id
                    transferText: (!sampleData && backend.connectedId === modelData.id) ? backend.transferText : ""
                    onConnectRequested: backend.connectTo(modelData.id)
                    onDisconnectRequested: backend.disconnect()
                    onSendRequested: picker.open()
                    // The sample shows one computer with a still and the rest
                    // with their monitor icon.
                    preview: sampleData ? (index === 0 ? "qrc:/sample/preview.jpg" : "")
                                        : (backend.previews[modelData.id] || "")
                    previewLimit: grid.previewLimit
                }
            }
            Item { Layout.columnSpan: grid.columns; Layout.preferredHeight: Theme.margin - Theme.gap }
        }
    }

    // What went wrong with the last action, and a file that arrived here, along
    // the bottom edge.
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        visible: !sampleData && (backend.message.length > 0 || backend.receivedText.length > 0)
        height: notes.implicitHeight + 16
        color: Theme.surface
        Rectangle { width: parent.width; height: 1; color: Theme.border }
        Column {
            id: notes
            anchors.verticalCenter: parent.verticalCenter
            x: Theme.margin
            width: parent.width - 2 * Theme.margin
            spacing: 2
            Text {
                width: parent.width
                visible: !sampleData && backend.message.length > 0
                text: backend.message
                color: Theme.orange
                font.pixelSize: Theme.smallTextSize
                wrapMode: Text.WordWrap
            }
            Text {
                width: parent.width
                visible: !sampleData && backend.receivedText.length > 0
                text: backend.receivedText
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
                elide: Text.ElideRight
            }
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        visible: page.computers.length === 0
        spacing: 6
        Glyph { Layout.alignment: Qt.AlignHCenter; icon: "monitor"; size: 32 }
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: !sampleData && !backend.background ? "Starting Secpar on this computer" : "No computers yet"
            color: Theme.text
            font.pixelSize: Theme.textSize
            font.weight: Font.DemiBold
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: !sampleData && !backend.background
                  ? "Its background part is starting. The list appears when it answers."
                  : "Sign in on another computer to see it here."
            color: Theme.textDim
            font.pixelSize: Theme.smallTextSize
        }
    }
}
