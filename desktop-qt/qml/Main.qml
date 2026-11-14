import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

ApplicationWindow {
    id: window
    width: 800
    height: 600
    minimumWidth: 640
    minimumHeight: 400
    visible: true
    title: "Secpar"
    color: Theme.background

    property int tab: Math.max(0, Math.min(2, startTab))

    header: Rectangle {
        height: 48
        color: Theme.background
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.margin
            anchors.rightMargin: Theme.margin
            spacing: 0
            // The wordmark file already includes the triangle.
            Image {
                source: "qrc:/branding/secpar-wordmark-light.svg"
                sourceSize.height: 18
                fillMode: Image.PreserveAspectFit
            }
            Text {
                Layout.leftMargin: 8
                Layout.rightMargin: Theme.margin
                text: appVersion
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
            }
            Repeater {
                model: ["Computers", "Friends", "Settings"]
                TopTab {
                    required property string modelData
                    required property int index
                    text: modelData
                    current: window.tab === index
                    onClicked: window.tab = index
                }
            }
            Item { Layout.fillWidth: true }
            Text {
                visible: sampleData
                text: "Layout sample"
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
            }
        }
        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: Theme.border
        }
    }

    StackLayout {
        anchors.fill: parent
        currentIndex: window.tab
        ComputersPage {}
        FriendsPage {}
        SettingsPage {}
    }
}
