import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

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
    readonly property var computers: sampleData ? sample : []

    ScrollView {
        anchors.fill: parent
        contentWidth: availableWidth
        visible: page.computers.length > 0

        GridLayout {
            id: grid
            readonly property int minimumTile: 280
            // Keep two rows of tiles on screen without scrolling when the window allows it.
            readonly property int previewLimit: Math.max(72, Math.floor((page.height - 2 * Theme.margin - Theme.gap) / 2) - 58)
            x: Theme.margin
            y: Theme.margin
            width: page.width - 2 * Theme.margin
            columns: Math.max(2, Math.floor((width + Theme.gap) / (minimumTile + Theme.gap)))
            columnSpacing: Theme.gap
            rowSpacing: Theme.gap

            Repeater {
                model: page.computers
                ComputerTile {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    Layout.alignment: Qt.AlignTop
                    name: modelData.name
                    owner: modelData.owner
                    online: modelData.online
                    route: modelData.route
                    previewLimit: grid.previewLimit
                }
            }
            Item { Layout.columnSpan: grid.columns; Layout.preferredHeight: Theme.margin - Theme.gap }
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        visible: page.computers.length === 0
        spacing: 6
        Glyph { Layout.alignment: Qt.AlignHCenter; icon: "monitor"; size: 32 }
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "No computers yet"
            color: Theme.text
            font.pixelSize: Theme.textSize
            font.weight: Font.DemiBold
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "Sign in on another computer to see it here."
            color: Theme.textDim
            font.pixelSize: Theme.smallTextSize
        }
    }
}
