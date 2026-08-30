import QtQuick
import QtQuick.Layouts

Item {
    id: root

    property string title: ""
    property string detail: ""
    property bool checked: false
    property bool controlEnabled: true
    signal requested(bool checked)

    Layout.fillWidth: true
    implicitHeight: Math.max(copy.implicitHeight, toggle.implicitHeight)
    opacity: controlEnabled ? 1 : 0.62

    RowLayout {
        anchors.fill: parent
        spacing: 16

        ColumnLayout {
            id: copy
            Layout.fillWidth: true
            spacing: 2

            Text {
                Layout.fillWidth: true
                text: root.title
                color: BadiTheme.text
                font.pixelSize: 13
                font.weight: Font.Medium
                wrapMode: Text.WordWrap
            }

            Text {
                Layout.fillWidth: true
                text: root.detail
                color: BadiTheme.textDim
                font.pixelSize: 11
                lineHeight: 1.2
                wrapMode: Text.WordWrap
            }
        }

        ToggleControl {
            id: toggle
            text: root.title
            checked: root.checked
            enabled: root.controlEnabled
            Accessible.description: root.detail
            onRequested: checked => root.requested(checked)
        }
    }
}
