import QtQuick
import QtQuick.Controls.impl

// One Lucide icon (assets/icons/lucide), tinted. Icons are never drawn by hand.
IconImage {
    property string icon
    property int size: 16
    source: "qrc:/icons/lucide/" + icon + ".svg"
    sourceSize: Qt.size(size, size)
    width: size
    height: size
    color: Theme.textDim
}
