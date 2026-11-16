#!/bin/sh
# Secpar: the window. Connects to computers and controls hosting on this one.
here=$(cd -- "$(dirname -- "$0")" && pwd)
export LD_LIBRARY_PATH="$here:$here/lib:$here/engine/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export QT_PLUGIN_PATH="$here/qt6/plugins${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}"
export QML2_IMPORT_PATH="$here/qt6/qml${QML2_IMPORT_PATH:+:$QML2_IMPORT_PATH}"
export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-wayland;xcb}"
exec "$here/secpar" "$@"
