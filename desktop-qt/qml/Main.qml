import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qt.labs.platform as Platform

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
    // The layout sample shows the pages as they look when signed in.
    readonly property bool signedIn: sampleData || backend.signedIn
    // Set while the app is really ending, so closing the window can hide it instead.
    property bool quitting: false

    function showWindow() {
        window.show()
        window.raise()
        window.requestActivate()
    }
    function quitApp() {
        window.quitting = true
        Qt.quit()
    }
    // Closing the window leaves Secpar in the tray, with the background part and
    // any stream still running. Without a tray, closing ends the app as usual.
    onClosing: function (close) {
        if (!window.quitting && tray.available) {
            close.accepted = false
            window.hide()
        }
    }

    Platform.SystemTrayIcon {
        id: tray
        visible: true
        icon.source: "qrc:/branding/secpar-icon.svg"
        tooltip: "Secpar"
        menu: Platform.Menu {
            Platform.MenuItem {
                text: "Open Secpar"
                onTriggered: window.showWindow()
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: "Quit"
                onTriggered: window.quitApp()
            }
        }
        onActivated: function (reason) {
            if (reason === Platform.SystemTrayIcon.Trigger || reason === Platform.SystemTrayIcon.DoubleClick)
                window.showWindow()
        }
    }

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
                model: window.signedIn ? ["Computers", "Friends", "Settings"] : []
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
        visible: window.signedIn
        currentIndex: window.tab
        ComputersPage {}
        FriendsPage {}
        SettingsPage {}
    }
    SignInPage {
        anchors.fill: parent
        visible: !window.signedIn && backend.loaded
    }
}
