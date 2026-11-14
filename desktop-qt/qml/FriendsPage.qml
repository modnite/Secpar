import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

// Friends, one row each: name, whether they are trusted, and what they may use.
Item {
    id: page
    readonly property var sample: [
        { name: "Mika", trusted: true, access: "Controller, keyboard and mouse" },
        { name: "Jo", trusted: true, access: "Controller" },
        { name: "Sam", trusted: false, access: "Asks each time" }
    ]
    readonly property var friends: sampleData ? sample : []

    ListView {
        anchors.fill: parent
        anchors.margins: Theme.margin
        visible: page.friends.length > 0
        clip: true
        spacing: 0
        model: page.friends
        delegate: Rectangle {
            required property var modelData
            required property int index
            width: ListView.view.width
            height: 56
            color: "transparent"
            Rectangle {
                anchors.bottom: parent.bottom
                width: parent.width
                height: 1
                color: Theme.border
            }
            RowLayout {
                anchors.fill: parent
                spacing: Theme.gap
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Text {
                        text: modelData.name
                        color: Theme.text
                        font.pixelSize: Theme.textSize
                        font.weight: Font.DemiBold
                    }
                    Text {
                        Layout.fillWidth: true
                        text: modelData.access
                        color: Theme.textDim
                        font.pixelSize: Theme.smallTextSize
                        elide: Text.ElideRight
                    }
                }
                Text {
                    text: modelData.trusted ? "Trusted" : "Not trusted"
                    color: modelData.trusted ? Theme.mint : Theme.textDim
                    font.pixelSize: Theme.smallTextSize
                }
                FlatButton { text: "Permissions" }
            }
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        visible: page.friends.length === 0
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
            text: "Friends you add can ask to join your computers."
            color: Theme.textDim
            font.pixelSize: Theme.smallTextSize
        }
    }
}
