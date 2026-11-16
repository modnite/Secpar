import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

// Sections on the left, the chosen section's rows on the right. A row whose value
// is a choice opens a dropdown instead of changing on click.
Item {
    id: page
    property int section: startSection === 2 ? 1 : Math.max(0, Math.min(5, startSection))
    readonly property var sections: ["Hosting", "Connecting", "Controllers", "Account", "Devices", "About"]
    // Fictional rows for layout review only (--sample). A third entry names the
    // choice list the row uses, so the sample shows the same dropdowns.
    readonly property var sampleRows: [
        [["Host this computer", "On"], ["Players", "Up to 3 devices"],
         ["Allowed devices", "2"], ["Access list", "20 Nov 2026 18:00"]],
        [["Resolution", "", "resolution"], ["Frame rate", "", "fps"], ["Bitrate", "", "bitrate_kbps"],
         ["Decoder", "", "decoder"], ["Display", "", "display"], ["Audio", "", "audio"],
         ["Mute when focus is lost", "", "mute_on_focus_loss"], ["Share clipboard text", "", "share_clipboard"]],
        [["Rumble", "", "rumble"], ["Motion sensors", "", "motion"], ["Touchpad", "", "touchpad"]],
        [["Signed in as", "Layout sample"], ["Two-step sign-in", "", "two_step"]],
        [["This device", "Layout sample"]],
        [["Version", appVersion], ["Licence", "GPL-3.0"]]
    ]
    // The sample's own starting values, used only while --sample is on.
    property var sampleValues: ({
        resolution: "1920x1080", fps: 60, bitrate_kbps: 20000, decoder: "auto",
        display: "windowed", audio: "stereo", mute_on_focus_loss: false,
        share_clipboard: true, rumble: true, motion: true, touchpad: true, two_step: false
    })
    readonly property var platforms: ({ windows: "Windows", linux: "Linux", android: "Android" })
    readonly property var accountRows: [["Signed in as", backend.handle]]
    readonly property var deviceRows: backend.devices.map(d => [
        d.this ? d.name + " (this computer)" : d.name,
        (platforms[d.platform] || d.platform) + (d.hosts ? ", hosts" : ""),
        d.id, d.this])
    // How many names have more than one record. Removing takes the extras and
    // never this computer, which is what the stale records from old versions need.
    readonly property int duplicateNames: page.countDuplicates()
    function countDuplicates() {
        const seen = [], repeated = []
        for (const device of backend.devices) {
            const key = String(device.name).trim().toLowerCase()
            if (seen.indexOf(key) < 0)
                seen.push(key)
            else if (repeated.indexOf(key) < 0)
                repeated.push(key)
        }
        return repeated.length
    }
    // The stream component's accepted values, and the two switches beside them.
    readonly property var choices: ({
        resolution: [["1280x720", "1280 × 720"], ["1920x1080", "1920 × 1080"],
                     ["2560x1440", "2560 × 1440"], ["3840x2160", "3840 × 2160"]],
        fps: [[30, "30 fps"], [60, "60 fps"], [120, "120 fps"]],
        bitrate_kbps: [[10000, "10 Mbps"], [20000, "20 Mbps"],
                       [40000, "40 Mbps"], [80000, "80 Mbps"]],
        decoder: [["auto", "Automatic"], ["hardware", "Hardware"], ["software", "Software"]],
        display: [["windowed", "Windowed"], ["fullscreen", "Fullscreen"], ["borderless", "Borderless"]],
        audio: [["stereo", "Stereo"], ["5.1-surround", "5.1 surround"], ["7.1-surround", "7.1 surround"]],
        mute_on_focus_loss: [[true, "On"], [false, "Off"]],
        share_clipboard: [[true, "On"], [false, "Off"]]
    })
    // Controller rows exist in the sample only. The stream component owns them.
    readonly property var sampleChoices: ({
        rumble: [[true, "On"], [false, "Off"]],
        motion: [[true, "On"], [false, "Off"]],
        touchpad: [[true, "On"], [false, "Off"]],
        two_step: [[true, "On"], [false, "Off"]]
    })
    function optionsFor(key) {
        return page.choices[key] || page.sampleChoices[key] || []
    }
    function currentValue(key) {
        return sampleData ? page.sampleValues[key] : backend.preferences[key]
    }
    function labelFor(key, value) {
        const found = page.optionsFor(key).find(choice => choice[0] === value)
        return found ? found[1] : ""
    }
    function rowIsChoice(row) {
        return row.length > 2 && page.optionsFor(row[2]).length > 0
    }
    function rowValue(row) {
        return page.rowIsChoice(row) ? page.labelFor(row[2], page.currentValue(row[2])) : row[1]
    }
    function openChooser(anchor, key, restore) {
        const list = page.optionsFor(key)
        if (list.length === 0)
            return
        chooser.openFor(anchor, key, list, page.currentValue(key), page, restore)
    }
    function pick(key, value) {
        if (sampleData) {
            const next = Object.assign({}, page.sampleValues)
            next[key] = value
            page.sampleValues = next
        } else {
            backend.setPreference(key, value)
        }
    }
    readonly property var hostingRows: [
        ["Host this computer", backend.hosting ? "On" : (backend.background ? "Off" : "Starting")],
        ["Players", "Up to 3 devices"],
        ["Allowed devices", String(backend.allowedDevices)],
        ["Access list", backend.accessList]
    ]
    readonly property var connectingRows: [
        ["Resolution", "", "resolution"],
        ["Frame rate", "", "fps"],
        ["Bitrate", "", "bitrate_kbps"],
        ["Decoder", "", "decoder"],
        ["Display", "", "display"],
        ["Audio", "", "audio"],
        ["Mute when focus is lost", "", "mute_on_focus_loss"],
        ["Share clipboard text", "", "share_clipboard"]
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
                    // Controllers has no options of its own yet, so it is not offered.
                    visible: entry.index !== 2
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
                        border.width: entry.visualFocus ? 1 : 0
                        border.color: Theme.text
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
                    id: rowItem
                    required property var modelData
                    required property int index
                    readonly property bool choosable: page.rowIsChoice(modelData)
                    Layout.fillWidth: true
                    // Eight rows still fit without scrolling at 640x400.
                    implicitHeight: 36
                    color: (rowMouse.containsMouse && rowMouse.enabled) || rowItem.activeFocus
                           ? Theme.surface : "transparent"
                    activeFocusOnTab: rowItem.choosable
                    Keys.onPressed: function (event) {
                        if (!rowItem.choosable)
                            return
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter
                                || event.key === Qt.Key_Space || event.key === Qt.Key_Down) {
                            page.openChooser(valueControl, rowItem.modelData[2], rowItem)
                            event.accepted = true
                        }
                    }
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                    // Clicking anywhere on a choice row opens its list.
                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        enabled: rowItem.choosable
                        hoverEnabled: true
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: page.openChooser(valueControl, rowItem.modelData[2], rowItem)
                    }
                    RowLayout {
                        anchors.fill: parent
                        Text {
                            Layout.fillWidth: true
                            text: rowItem.modelData[0]
                            color: Theme.text
                            font.pixelSize: Theme.textSize
                            elide: Text.ElideRight
                        }
                        Text {
                            visible: !rowItem.choosable
                            text: rowItem.modelData[1]
                            color: Theme.textDim
                            font.pixelSize: Theme.textSize
                            elide: Text.ElideRight
                        }
                        ValueButton {
                            id: valueControl
                            visible: rowItem.choosable
                            width: implicitWidth
                            text: page.rowValue(rowItem.modelData)
                            rowFocus: rowItem.activeFocus
                            onClicked: page.openChooser(valueControl, rowItem.modelData[2], rowItem)
                        }
                        FlatButton {
                            visible: page.section === 4 && !sampleData && rowItem.modelData.length > 2 && !rowItem.modelData[3]
                            text: "Remove"
                            onClicked: backend.removeDevice(rowItem.modelData[2])
                        }
                    }
                    // --sample with --sample-open: the first choice row shows its
                    // list, so a review capture can include it.
                    Timer {
                        running: sampleData && sampleOpen && page.section === 1
                                 && rowItem.index === 0 && rowItem.choosable
                        interval: 250
                        onTriggered: page.openChooser(valueControl, rowItem.modelData[2], rowItem)
                    }
                }
            }
            // The release feed, in one row. Installing ends a stream, so the
            // row asks once before it starts.
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 36
                visible: page.section === 5
                color: "transparent"
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.gap
                    Text {
                        Layout.fillWidth: true
                        text: "Updates"
                        color: Theme.text
                        font.pixelSize: Theme.textSize
                        elide: Text.ElideRight
                    }
                    Text {
                        text: backend.updateText
                        color: Theme.textDim
                        font.pixelSize: Theme.textSize
                        elide: Text.ElideRight
                    }
                    FlatButton {
                        visible: !backend.installing && !backend.updateConfirming
                        text: backend.updateAvailable ? "Install v" + backend.updateVersion
                                                      : "Check for updates"
                        enabled: !backend.busy
                        onClicked: backend.updateAvailable ? backend.updateConfirming = true
                                                           : backend.checkForUpdates()
                    }
                    FlatButton {
                        visible: backend.updateConfirming
                        primary: true
                        text: "Install"
                        enabled: !backend.installing
                        onClicked: backend.installUpdate()
                    }
                    FlatButton {
                        visible: backend.updateConfirming
                        text: "Cancel"
                        onClicked: backend.cancelUpdate()
                    }
                }
            }
            Text {
                Layout.fillWidth: true
                Layout.topMargin: 6
                visible: page.section === 5 && backend.updateConfirming
                text: "Installing restarts Secpar and ends any stream."
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
                wrapMode: Text.WordWrap
            }
            FlatButton {
                Layout.topMargin: Theme.gap
                visible: page.section === 4 && !sampleData
                text: "Remove duplicate computers"
                enabled: page.duplicateNames > 0 && !backend.busy
                onClicked: backend.removeDuplicateDevices()
            }
            Text {
                Layout.fillWidth: true
                Layout.topMargin: 6
                visible: page.section === 4 && !sampleData
                text: "Keeps one record for each computer name. This computer is always kept."
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
                wrapMode: Text.WordWrap
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
                visible: !sampleData && page.section === 2 && page.rows.length === 0
                text: "Controller options are set by the stream component."
                color: Theme.textDim
                font.pixelSize: Theme.smallTextSize
                wrapMode: Text.WordWrap
            }
            Text {
                Layout.fillWidth: true
                Layout.topMargin: 6
                visible: !sampleData && page.section === 4 && page.rows.length === 0
                text: "No devices on this account yet."
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

    SelectMenu {
        id: chooser
        onPicked: function (key, value) { page.pick(key, value) }
    }

    // --update-confirm: the update row shows the asking step, so a review
    // capture can include it.
    Timer {
        running: updateConfirmHint && page.section === 5 && backend.updateAvailable
        interval: 2500
        onTriggered: backend.updateConfirming = true
    }
}
