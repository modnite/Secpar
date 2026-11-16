cmake_minimum_required(VERSION 3.21)
# The release version lives in version.txt in the repository root: the updater,
# the Windows installer and the AppImage read the same file.
file(STRINGS "${CMAKE_CURRENT_SOURCE_DIR}/../version.txt" SECPAR_VERSION LIMIT_COUNT 1)
project(Secpar VERSION ${SECPAR_VERSION} LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.8 REQUIRED COMPONENTS Quick QuickControls2 Svg LabsPlatform Widgets)
qt_standard_project_setup(REQUIRES 6.8)

qt_add_executable(secpar WIN32 src/main.cpp src/backend.cpp src/backend.h)
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
        qml/SignInPage.qml
        qml/FlatField.qml
        qml/SelectMenu.qml
        qml/ValueButton.qml
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
        ${assets}/icons/lucide/chevron-down.svg
        ${assets}/icons/lucide/chevron-right.svg
)

# The still the layout sample shows in a computer tile (--sample only). It is a
# drawn desktop picture, not a capture of anyone's screen.
qt_add_resources(secpar "sample"
    PREFIX "/sample"
    BASE ${CMAKE_CURRENT_SOURCE_DIR}/sample
    FILES ${CMAKE_CURRENT_SOURCE_DIR}/sample/preview.jpg
)

# The app library built from crates/secpar-app (cargo build --release). Pass the
# folder that holds it, for example -DSECPAR_APP_DIR=<cargo target>/release.
set(SECPAR_APP_DIR "" CACHE PATH "Folder with the built secpar-app library")
find_library(SECPAR_APP_LIBRARY NAMES secpar_app.dll secpar_app PATHS "${SECPAR_APP_DIR}" NO_DEFAULT_PATH REQUIRED)
target_link_libraries(secpar PRIVATE Qt6::Quick Qt6::QuickControls2 Qt6::Svg Qt6::LabsPlatform Qt6::Widgets "${SECPAR_APP_LIBRARY}")
if(WIN32)
    add_custom_command(TARGET secpar POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different "${SECPAR_APP_DIR}/secpar_app.dll" "$<TARGET_FILE_DIR:secpar>")
endif()
