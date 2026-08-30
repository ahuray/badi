import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root

    property string title: ""
    property string subtitle: ""
    default property alias contentData: contentColumn.data

    Layout.fillWidth: true
    implicitHeight: cardLayout.implicitHeight + 36
    radius: BadiTheme.radiusLarge
    color: BadiTheme.surface
    border.width: 1
    border.color: BadiTheme.border

    ColumnLayout {
        id: cardLayout
        anchors.fill: parent
        anchors.margins: 18
        spacing: 14

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            visible: root.title.length > 0 || root.subtitle.length > 0

            Text {
                Layout.fillWidth: true
                text: root.title
                visible: root.title.length > 0
                color: BadiTheme.text
                font.pixelSize: 15
                font.weight: Font.DemiBold
            }

            Text {
                Layout.fillWidth: true
                text: root.subtitle
                visible: root.subtitle.length > 0
                color: BadiTheme.textMuted
                font.pixelSize: 12
                lineHeight: 1.25
                wrapMode: Text.WordWrap
            }
        }

        ColumnLayout {
            id: contentColumn
            Layout.fillWidth: true
            spacing: 10
        }
    }
}
