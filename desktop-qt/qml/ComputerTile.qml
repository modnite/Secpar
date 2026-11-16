import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

// One computer: preview on top, then its name, owner and state, and Connect.
Rectangle {
    id: tile
    property string name
    property string owner      // empty for the user's own computers
    property bool online
    property string route      // "This network", "Direct", "Relay" or empty
    property string status     // shown instead of the online state when set
    property url preview       // a still of its screen, when it sent one
    property int previewLimit: 10000   // tallest the preview may be
    property bool connected: false
    property bool streaming: false
    property bool busy: false
    // The row with "Send a file…" only exists while this computer is connected.
    property bool sendVisible: false
    // How far a file being sent to it has got, or that it was sent.
    property string transferText
    signal connectRequested()
    signal disconnectRequested()
    signal sendRequested()

    radius: Theme.radius
    color: Theme.surface
    border.width: 1
    border.color: Theme.border
    implicitHeight: column.implicitHeight

    ColumnLayout {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: 0

        // Preview, 16:9. A still of its screen once one has arrived; until then,
        // and on a computer that sends none, the monitor icon.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(Math.round(width * 9 / 16), tile.previewLimit)
            Layout.margins: 1
            topLeftRadius: Theme.radius - 1
            topRightRadius: Theme.radius - 1
            color: Theme.background
            clip: true
            Image {
                id: still
                anchors.fill: parent
                source: tile.preview
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
                visible: status === Image.Ready
            }
            Glyph {
                anchors.centerIn: parent
                visible: !still.visible
                icon: "monitor"
                size: 32
                color: tile.online ? Theme.textDim : Theme.border
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.gap
            spacing: Theme.gap
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Text {
                    Layout.fillWidth: true
                    text: tile.name
                    color: Theme.text
                    font.pixelSize: Theme.textSize
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                RowLayout {
                    spacing: 6
                    Rectangle {
                        width: 7; height: 7; radius: 4
                        color: tile.online ? Theme.mint : Theme.border
                    }
                    Text {
                        Layout.fillWidth: true
                        text: {
                            const state = tile.status || (tile.streaming ? "Streaming"
                                          : tile.online ? (tile.route || "Online") : "Offline")
                            return tile.owner ? tile.owner + " · " + state : state
                        }
                        color: Theme.textDim
                        font.pixelSize: Theme.smallTextSize
                        elide: Text.ElideRight
                    }
                }
            }
            FlatButton {
                text: tile.connected ? "Disconnect" : "Connect"
                primary: !tile.connected && enabled
                enabled: tile.connected || (tile.online && !tile.busy)
                visible: tile.connected || tile.online
                onClicked: tile.connected ? tile.disconnectRequested() : tile.connectRequested()
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.gap
            Layout.rightMargin: Theme.gap
            Layout.bottomMargin: Theme.gap
            spacing: Theme.gap
            visible: tile.sendVisible
            FlatButton {
                text: "Send a file…"
                // One file at a time towards this computer.
                enabled: tile.transferText.indexOf("Sending ") !== 0
                onClicked: tile.sendRequested()
            }
            Text {
                Layout.fillWidth: true
                text: tile.transferText
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
                elide: Text.ElideMiddle
                visible: tile.transferText.length > 0
            }
        }
    }
}
