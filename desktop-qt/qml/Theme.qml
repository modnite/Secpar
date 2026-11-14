pragma Singleton
import QtQuick

// Approved brand colours (assets/branding/README.md) and the shared measurements
// every page uses, so control heights and edges line up.
QtObject {
    readonly property color background: "#15161A"
    readonly property color surface: "#1C1E24"
    readonly property color surfaceRaised: "#252830"
    readonly property color border: "#2F333C"
    readonly property color text: "#F9FAFC"
    readonly property color textDim: "#9BA1AD"
    readonly property color mint: "#0AFFB6"
    readonly property color orange: "#D95400"

    readonly property int controlHeight: 32
    readonly property int margin: 16
    readonly property int gap: 12
    readonly property int radius: 6
    readonly property int textSize: 13
    readonly property int smallTextSize: 12
}
