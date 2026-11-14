import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

// Sections on the left, the chosen section's rows on the right.
Item {
    id: page
    property int section: 0
    readonly property var sections: ["Hosting", "Connecting", "Controllers", "Account", "Devices", "About"]
    // Fictional rows for layout review only (--sample).
    readonly property var sampleRows: [
        [["Host this computer", "On"], ["Resolution", "1920 × 1080"], ["Frame rate", "60 fps"],
         ["Players", "Up to 3 devices"], ["Display", "Primary"]],
        [["Decoder", "Hardware"], ["Resolution", "Match this screen"], ["Frame rate", "60 fps"],
         ["Release keyboard and mouse", "Ctrl + Alt + Z"]],
        [["Rumble", "On"], ["Motion sensors", "On"], ["Touchpad", "On"]],
        [["Signed in as", "Layout sample"], ["Two-step sign-in", "Off"]],
        [["This device", "Layout sample"]],
        []
    ]
    readonly property var rows: section === 5 ? [["Version", appVersion], ["Licence", "GPL-3.0"]]
                                              : (sampleData ? sampleRows[section] : [])

    RowLayout {
        anchors.fill: parent
        spacing: 0

        ColumnLayout {
            Layout.preferredWidth: 168
            Layout.maximumWidth: 168
            Layout.fillWidth: false
            Layout.fillHeight: true
            Layout.margins: Theme.margin
            Layout.rightMargin: 0
            spacing: 2
            Repeater {
                model: page.sections
                AbstractButton {
                    id: entry
                    required property string modelData
                    required property int index
                    Layout.fillWidth: true
                    implicitHeight: Theme.controlHeight
                    onClicked: page.section = index
                    contentItem: Text {
                        leftPadding: 10
                        text: entry.modelData
                        color: page.section === entry.index ? Theme.text : Theme.textDim
                        font.pixelSize: Theme.textSize
                        font.weight: page.section === entry.index ? Font.DemiBold : Font.Normal
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.radius
                        color: page.section === entry.index ? Theme.surfaceRaised
                                                            : (entry.hovered ? Theme.surface : "transparent")
                    }
                }
            }
            Item { Layout.fillHeight: true }
        }
        Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; Layout.leftMargin: Theme.margin; color: Theme.border }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: Theme.margin
            spacing: 0
            Text {
                text: page.sections[page.section]
                color: Theme.text
                font.pixelSize: 15
                font.weight: Font.DemiBold
                bottomPadding: Theme.gap
            }
            Repeater {
                model: page.rows
                Rectangle {
                    required property var modelData
                    Layout.fillWidth: true
                    implicitHeight: 40
                    color: "transparent"
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                    RowLayout {
                        anchors.fill: parent
                        Text {
                            Layout.fillWidth: true
                            text: modelData[0]
                            color: Theme.text
                            font.pixelSize: Theme.textSize
                        }
                        Text {
                            text: modelData[1]
                            color: Theme.textDim
                            font.pixelSize: Theme.textSize
                        }
                    }
                }
            }
            Text {
                visible: page.rows.length === 0
                text: "Sign in to change these settings."
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
            }
            Item { Layout.fillHeight: true }
        }
    }
}
