import QtQuick
import QtQuick.Controls

AbstractButton {
    id: root

    signal requested(bool checked)

    implicitWidth: 48
    implicitHeight: 28
    focusPolicy: Qt.StrongFocus
    checkable: false

    Accessible.name: text
    Accessible.role: Accessible.CheckBox
    Accessible.checkable: true
    Accessible.checked: checked

    onClicked: requested(!checked)

    background: Rectangle {
        radius: height / 2
        color: root.checked ? BadiTheme.accentSurface : BadiTheme.surfaceRaised
        border.width: root.activeFocus ? 2 : 1
        border.color: root.activeFocus
            ? BadiTheme.accent
            : root.checked
                ? BadiTheme.withAlpha(BadiTheme.accent, 0.75)
                : BadiTheme.borderStrong

        Rectangle {
            width: 20
            height: 20
            radius: 10
            y: 4
            x: root.checked ? parent.width - width - 4 : 4
            color: root.enabled
                ? root.checked ? BadiTheme.accentStrong : BadiTheme.textMuted
                : BadiTheme.textDim

            Behavior on x { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
            Behavior on color { ColorAnimation { duration: 100 } }
        }

        Behavior on color { ColorAnimation { duration: 100 } }
    }
}
