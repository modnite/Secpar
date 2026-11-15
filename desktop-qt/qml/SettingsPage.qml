import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

// Sections on the left, the chosen section's rows on the right.
Item {
    id: page
    property int section: Math.max(0, Math.min(5, startSection))
    readonly property var sections: ["Hosting", "Connecting", "Controllers", "Account", "Devices", "About"]
    // Fictional rows for layout review only (--sample).
    readonly property var sampleRows: [
        [["Host this computer", "On"], ["Players", "Up to 3 devices"],
         ["Allowed devices", "2"], ["Access list", "20 Nov 2026 18:00"]],
        [["Resolution", "1920 × 1080"], ["Frame rate", "60 fps"], ["Bitrate", "20 Mbps"],
         ["Decoder", "Automatic"], ["Display", "Windowed"], ["Audio", "Stereo"],
         ["Mute when focus is lost", "Off"]],
        [["Rumble", "On"], ["Motion sensors", "On"], ["Touchpad", "On"]],
        [["Signed in as", "Layout sample"], ["Two-step sign-in", "Off"]],
        [["This device", "Layout sample"]],
        []
    ]
    readonly property var platforms: ({ windows: "Windows", linux: "Linux", android: "Android" })
    readonly property var accountRows: [["Signed in as", backend.handle]]
    readonly property var deviceRows: backend.devices.map(d => [
        d.this ? d.name + " (this computer)" : d.name,
        (platforms[d.platform] || d.platform) + (d.hosts ? ", hosts" : "")])
    // The stream component's accepted values, in the order they cycle.
    readonly property var choices: ({
        resolution: [["1280x720", "1280 × 720"], ["1920x1080", "1920 × 1080"],
                     ["2560x1440", "2560 × 1440"], ["3840x2160", "3840 × 2160"]],
        fps: [[30, "30 fps"], [60, "60 fps"], [120, "120 fps"]],
        bitrate_kbps: [[10000, "10 Mbps"], [20000, "20 Mbps"],
                       [40000, "40 Mbps"], [80000, "80 Mbps"]],
        decoder: [["auto", "Automatic"], ["hardware", "Hardware"], ["software", "Software"]],
        display: [["windowed", "Windowed"], ["fullscreen", "Fullscreen"], ["borderless", "Borderless"]],
        audio: [["stereo", "Stereo"], ["5.1-surround", "5.1 surround"], ["7.1-surround", "7.1 surround"]],
        mute_on_focus_loss: [[true, "On"], [false, "Off"]]
    })
    function label(key) {
        const list = page.choices[key] || []
        const value = backend.preferences[key]
        const found = list.find(choice => choice[0] === value)
        return found ? found[1] : ""
    }
    function cycle(key) {
        const list = page.choices[key] || []
        if (list.length === 0)
            return
        const value = backend.preferences[key]
        let index = list.findIndex(choice => choice[0] === value)
        index = index < 0 ? 0 : (index + 1) % list.length
        backend.setPreference(key, list[index][0])
    }
    readonly property var hostingRows: [
        ["Host this computer", backend.hosting ? "On" : "Off"],
        ["Players", "Up to 3 devices"],
        ["Allowed devices", String(backend.allowedDevices)],
        ["Access list", backend.accessList]
    ]
    readonly property var connectingRows: [
        ["Resolution", page.label("resolution"), "resolution"],
        ["Frame rate", page.label("fps"), "fps"],
        ["Bitrate", page.label("bitrate_kbps"), "bitrate_kbps"],
        ["Decoder", page.label("decoder"), "decoder"],
        ["Display", page.label("display"), "display"],
        ["Audio", page.label("audio"), "audio"],
        ["Mute when focus is lost", page.label("mute_on_focus_loss"), "mute_on_focus_loss"]
    ]
    readonly property var rows: section === 5 ? [["Version", appVersion], ["Licence", "GPL-3.0"]]
                                              : sampleData ? sampleRows[section]
                                              : section === 0 ? hostingRows
                                              : section === 1 ? connectingRows
                                              : section === 3 ? accountRows
                                              : section === 4 ? deviceRows : []

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
                    color: rowMouse.containsMouse && rowMouse.enabled ? Theme.surface : "transparent"
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
                            color: modelData.length > 2 && !sampleData ? Theme.mint : Theme.textDim
                            font.pixelSize: Theme.textSize
                        }
                    }
                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        enabled: !sampleData && modelData.length > 2
                        hoverEnabled: true
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: page.cycle(modelData[2])
                    }
                }
            }
            FlatButton {
                Layout.topMargin: Theme.gap
                visible: page.section === 3 && !sampleData
                text: backend.busy ? "Signing out" : "Sign out"
                enabled: !backend.busy
                onClicked: backend.signOut()
            }
            Text {
                Layout.fillWidth: true
                Layout.topMargin: 6
                visible: page.section === 3 && !sampleData
                text: "Signing out takes this computer off your account."
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
                wrapMode: Text.WordWrap
            }
            Text {
                Layout.fillWidth: true
                Layout.topMargin: 6
                visible: !sampleData && page.rows.length === 0
                text: "Controller options are set by the stream component."
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
                wrapMode: Text.WordWrap
            }
            Text {
                Layout.fillWidth: true
                Layout.topMargin: 8
                visible: backend.message.length > 0
                text: backend.message
                color: Theme.orange
                font.pixelSize: Theme.smallTextSize
                wrapMode: Text.WordWrap
            }
            Item { Layout.fillHeight: true }
        }
    }
}
