import QtQuick
import QtQuick.Layouts

Item {
    id: root

    property string title: ""
    property string detail: ""
    property string value: ""
    property string tone: "neutral"

    Layout.fillWidth: true
    implicitHeight: Math.max(copy.implicitHeight, valueText.implicitHeight)

    Accessible.name: title + (value.length > 0 ? ": " + value : "")
    Accessible.description: detail
    Accessible.role: Accessible.StaticText

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
                visible: root.detail.length > 0
                color: BadiTheme.textDim
                font.pixelSize: 11
                lineHeight: 1.2
                wrapMode: Text.WordWrap
            }
        }

        Text {
            id: valueText
            Layout.maximumWidth: Math.min(260, root.width * 0.42)
            text: root.value
            visible: root.value.length > 0
            color: BadiTheme.toneForeground(root.tone)
            font.pixelSize: 12
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignRight
            wrapMode: Text.Wrap
        }
    }
}
