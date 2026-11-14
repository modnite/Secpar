cmake_minimum_required(VERSION 3.21)
project(Secpar VERSION 4.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.8 REQUIRED COMPONENTS Quick QuickControls2 Svg)
qt_standard_project_setup(REQUIRES 6.8)

qt_add_executable(secpar WIN32 src/main.cpp)
target_compile_definitions(secpar PRIVATE SECPAR_VERSION="${PROJECT_VERSION}")

set_source_files_properties(qml/Theme.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)
qt_add_qml_module(secpar
    URI Secpar
    QML_FILES
        qml/Main.qml
        qml/Theme.qml
        qml/TopTab.qml
        qml/FlatButton.qml
        qml/Glyph.qml
        qml/ComputerTile.qml
        qml/ComputersPage.qml
        qml/FriendsPage.qml
        qml/SettingsPage.qml
)

# Approved brand art and the pinned Lucide icons, straight from the repository.
set(assets ${CMAKE_CURRENT_SOURCE_DIR}/../assets)
qt_add_resources(secpar "assets"
    PREFIX "/"
    BASE ${assets}
    FILES
        ${assets}/branding/secpar-icon.svg
        ${assets}/branding/secpar-wordmark-light.svg
        ${assets}/icons/lucide/monitor.svg
        ${assets}/icons/lucide/play.svg
        ${assets}/icons/lucide/check.svg
)

target_link_libraries(secpar PRIVATE Qt6::Quick Qt6::QuickControls2 Qt6::Svg)
