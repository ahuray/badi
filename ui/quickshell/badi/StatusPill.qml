import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root

    property string text: ""
    property string tone: "neutral"

    implicitWidth: pillLayout.implicitWidth + 18
    implicitHeight: 26
    radius: height / 2
    color: BadiTheme.toneBackground(tone)
    border.width: 1
    border.color: BadiTheme.withAlpha(BadiTheme.toneForeground(tone), 0.35)

    Accessible.name: text
    Accessible.role: Accessible.StaticText

    RowLayout {
        id: pillLayout
        anchors.centerIn: parent
        spacing: 7

        Rectangle {
            implicitWidth: 6
            implicitHeight: 6
            radius: 3
            color: BadiTheme.toneForeground(root.tone)
        }

        Text {
            text: root.text
            color: BadiTheme.toneForeground(root.tone)
            font.pixelSize: 11
            font.weight: Font.DemiBold
        }
    }
}
