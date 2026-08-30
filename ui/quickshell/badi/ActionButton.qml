import QtQuick
import QtQuick.Controls

Button {
    id: root

    property string tone: "neutral"
    property bool selected: false
    property bool selectionControl: false

    implicitHeight: 36
    implicitWidth: Math.max(86, contentItem.implicitWidth + 28)
    leftPadding: 14
    rightPadding: 14
    focusPolicy: Qt.StrongFocus

    Accessible.name: text
    Accessible.role: selectionControl ? Accessible.RadioButton : Accessible.Button
    Accessible.checkable: selectionControl
    Accessible.checked: selectionControl && selected

    contentItem: Text {
        text: root.text
        color: root.enabled
            ? (root.tone === "danger" ? BadiTheme.danger
               : root.selected || root.tone === "accent" ? BadiTheme.accentStrong
               : BadiTheme.text)
            : BadiTheme.textDim
        font.pixelSize: 12
        font.weight: Font.DemiBold
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    background: Rectangle {
        radius: BadiTheme.radiusSmall
        color: root.down
            ? BadiTheme.surfaceHover
            : root.selected || root.tone === "accent"
                ? BadiTheme.accentSurface
                : root.hovered
                    ? BadiTheme.surfaceRaised
                    : "transparent"
        border.width: root.activeFocus ? 2 : 1
        border.color: root.activeFocus || root.selected || root.tone === "accent"
            ? BadiTheme.accent
            : root.tone === "danger"
                ? BadiTheme.withAlpha(BadiTheme.danger, 0.55)
                : BadiTheme.border

        Behavior on color { ColorAnimation { duration: 100 } }
        Behavior on border.color { ColorAnimation { duration: 100 } }
    }
}
