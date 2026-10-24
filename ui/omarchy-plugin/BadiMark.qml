import QtQuick

Item {
  id: root
  property color ink: "white"

  Rectangle {
    anchors.fill: parent
    anchors.bottomMargin: parent.height * 0.12
    radius: width * 0.23
    color: "transparent"
    border.color: root.ink
    border.width: Math.max(1, width * 0.07)
    Text {
      anchors.centerIn: parent
      anchors.verticalCenterOffset: -parent.height * 0.04
      text: "b"
      font.family: "sans-serif"
      font.bold: true
      font.pixelSize: parent.height * 0.86
      color: root.ink
    }
  }
  Rectangle {
    x: parent.width * 0.21
    y: parent.height * 0.77
    width: parent.width * 0.10
    height: parent.height * 0.23
    color: root.ink
    rotation: 30
  }
}
