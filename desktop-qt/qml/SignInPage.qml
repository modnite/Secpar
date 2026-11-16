import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

// Shown instead of the tabs until someone is signed in on this computer.
Item {
    id: page
    function submit() {
        if (email.text.length > 0 && password.text.length > 0 && !backend.busy) {
            backend.signIn(email.text, password.text)
            password.clear()
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: 280
        spacing: Theme.gap

        Text {
            text: "Sign in"
            color: Theme.text
            font.pixelSize: 15
            font.weight: Font.DemiBold
        }
        FlatField {
            id: email
            Layout.fillWidth: true
            placeholderText: "Email"
            inputMethodHints: Qt.ImhEmailCharactersOnly
            focus: true
            onAccepted: password.forceActiveFocus()
        }
        FlatField {
            id: password
            Layout.fillWidth: true
            placeholderText: "Password"
            echoMode: TextInput.Password
            onAccepted: page.submit()
        }
        FlatButton {
            Layout.fillWidth: true
            text: backend.busy ? "Signing in" : "Sign in"
            primary: true
            enabled: !backend.busy && email.text.length > 0 && password.text.length > 0
            onClicked: page.submit()
        }
        Text {
            Layout.fillWidth: true
            visible: backend.message.length > 0
            text: backend.message
            color: Theme.orange
            font.pixelSize: Theme.smallTextSize
            wrapMode: Text.WordWrap
        }
        Text {
            Layout.fillWidth: true
            text: "This computer is added to your account when you sign in."
            color: Theme.textDim
            font.pixelSize: Theme.smallTextSize
            wrapMode: Text.WordWrap
        }
    }
}
