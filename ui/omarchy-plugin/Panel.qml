import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import qs.Ui
import qs.Commons

Item {
  id: root

  property var shell: null
  property var manifest: null
  property bool opened: false
  property bool closingFromHost: false
  property bool confirmMemoryClear: false

  readonly property string pluginId: manifest && manifest.id
    ? String(manifest.id) : "io.github.ahuray.badi"
  readonly property color foreground: Color.popups.text
  readonly property color background: Color.popups.background
  readonly property color accent: Color.accent
  readonly property color urgent: Color.urgent

  function open(payloadJson) {
    // The host payload is intentionally ignored. Plugin payload is never
    // executable input and cannot influence argv construction.
    root.closingFromHost = false
    root.opened = true
    window.visible = true
    client.activate()
    Qt.callLater(function() {
      if (window.visible) refreshButton.forceActiveFocus()
    })
  }

  function close() {
    root.closingFromHost = true
    root.opened = false
    root.confirmMemoryClear = false
    client.deactivate()
    window.visible = false
    root.closingFromHost = false
  }

  function requestClose() {
    if (root.shell && typeof root.shell.hide === "function")
      root.shell.hide(root.pluginId)
    else close()
  }

  function byteLabel(value) {
    if (!Number.isSafeInteger(value) || value < 0) return "Not reported"
    if (value < 1024) return value + " B"
    return (value / 1024).toFixed(1) + " KiB"
  }

  function authorityLabel() {
    if (!client.settingsDocumentValid) return "Unavailable"
    if (client.targetBundleAllowed) return "Allowed"
    if (client.targetAnyAuthority) return "Partial · fail closed"
    return "Blocked"
  }

  BadiClient {
    id: client
    onRefreshed: root.confirmMemoryClear = false
  }

  FloatingWindow {
    id: window
    title: "Badi controls"
    visible: false
    color: root.background
    implicitWidth: Style.space(560)
    implicitHeight: Style.space(700)
    minimumSize: Qt.size(Style.space(440), Style.space(480))

    onVisibleChanged: {
      if (!visible && root.opened && !root.closingFromHost) root.requestClose()
    }

    FocusScope {
      anchors.fill: parent
      focus: true
      Keys.onEscapePressed: root.requestClose()

      ColumnLayout {
        anchors.fill: parent
        anchors.margins: Style.space(20)
        spacing: Style.space(14)

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)

          ColumnLayout {
            Layout.fillWidth: true
            spacing: Style.space(2)

            Text {
              textFormat: Text.PlainText
              text: "Badi"
              color: root.foreground
              font.family: Style.font.family
              font.pixelSize: Style.font.title
              font.bold: true
            }

            Text {
              textFormat: Text.PlainText
              text: "Local privacy and runtime authority"
              color: Qt.darker(root.foreground, 1.4)
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
            }
          }

          Button {
            id: refreshButton
            text: client.loading ? "Refreshing…" : "Refresh"
            iconText: "󰑐"
            focusable: true
            bordered: true
            enabled: !client.busy
            foreground: root.foreground
            onClicked: client.refresh()
            Accessible.name: "Refresh Badi status"
          }

          PanelActionButton {
            iconText: "󰅖"
            tooltipText: "Close"
            focusable: true
            foreground: root.foreground
            onClicked: root.requestClose()
            Accessible.name: "Close Badi controls"
          }
        }

        PanelSeparator {
          Layout.fillWidth: true
          foreground: root.foreground
        }

        ScrollView {
          id: scroll
          Layout.fillWidth: true
          Layout.fillHeight: true
          clip: true
          ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

          ColumnLayout {
            width: scroll.availableWidth
            spacing: Style.space(12)

            BorderSurface {
              Layout.fillWidth: true
              implicitHeight: brokerContent.implicitHeight + Style.space(24)
              color: Style.normalFillFor(root.foreground, root.accent)
              radius: Style.cornerRadius
              borderSpec: Border.controlSpec("normal", root.foreground, root.accent)

              ColumnLayout {
                id: brokerContent
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.margins: Style.space(12)
                spacing: Style.space(8)

                PanelSectionHeader {
                  text: "BROKER"
                  foreground: root.foreground
                }

                RowLayout {
                  Layout.fillWidth: true

                  Text {
                    textFormat: Text.PlainText
                    text: "Runtime"
                    color: root.foreground
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                  }

                  Item { Layout.fillWidth: true }

                  Text {
                    textFormat: Text.PlainText
                    text: !client.brokerReachable
                      ? "Unavailable"
                      : client.controlPlaneDegraded
                        ? "Fail closed"
                        : client.brokerPaused
                          ? "Paused"
                          : "Active"
                    color: !client.brokerReachable || client.controlPlaneDegraded
                      ? root.urgent : client.brokerPaused
                        ? Qt.darker(root.foreground, 1.35) : root.accent
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                    font.bold: true
                  }
                }

                Text {
                  Layout.fillWidth: true
                  textFormat: Text.PlainText
                  text: client.authorityEpoch >= 0 && client.brokerSettingsRevision >= 0
                    ? "Authority epoch " + client.authorityEpoch
                      + " · settings revision " + client.brokerSettingsRevision
                    : "Authority counters are not reported."
                  color: Qt.darker(root.foreground, 1.4)
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  wrapMode: Text.WordWrap
                }
              }
            }

            BorderSurface {
              Layout.fillWidth: true
              implicitHeight: permissionContent.implicitHeight + Style.space(24)
              color: Style.normalFillFor(root.foreground, root.accent)
              radius: Style.cornerRadius
              borderSpec: Border.controlSpec("normal", root.foreground, root.accent)

              ColumnLayout {
                id: permissionContent
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.margins: Style.space(12)
                spacing: Style.space(8)

                PanelSectionHeader {
                  text: "DILLINGER PRODUCT PATH"
                  foreground: root.foreground
                }

                RowLayout {
                  Layout.fillWidth: true

                  ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Style.space(2)

                    Text {
                      textFormat: Text.PlainText
                      text: "https://dillinger.io/"
                      color: root.foreground
                      font.family: Style.font.family
                      font.pixelSize: Style.font.body
                      font.bold: true
                    }

                    Text {
                      Layout.fillWidth: true
                      textFormat: Text.PlainText
                      text: "Exact top-level document · HTTPS origin authority"
                      color: Qt.darker(root.foreground, 1.4)
                      font.family: Style.font.family
                      font.pixelSize: Style.font.caption
                    }
                  }

                  Text {
                    textFormat: Text.PlainText
                    text: root.authorityLabel()
                    color: client.targetBundleAllowed ? root.accent
                      : client.targetAnyAuthority ? root.urgent : root.foreground
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                    font.bold: true
                  }
                }

                Text {
                  Layout.fillWidth: true
                  textFormat: Text.PlainText
                  text: !client.memoryStoreAvailableReported
                    ? "Memory condition is not reported. Block all remains fail-closed; grants stay disabled."
                    : !client.memoryStoreAvailable
                      ? "Unreadable aggregate evidence is preserved. Block all writes a durable deny before acknowledgement; Allow stays disabled until explicit repair."
                      : "Chromium accepts only the exact Dillinger document. The broker stores authority for its HTTPS origin."
                  color: !client.memoryStoreAvailable ? root.urgent
                    : Qt.darker(root.foreground, 1.4)
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  wrapMode: Text.WordWrap
                }

                RowLayout {
                  Layout.fillWidth: true
                  spacing: Style.space(8)

                  Button {
                    text: "Block all"
                    iconText: "󰌾"
                    focusable: true
                    bordered: true
                    enabled: client.canRevokeSubjects && client.targetAnyAuthority
                    foreground: root.urgent
                    onClicked: client.blockTarget()
                    Accessible.name: "Block all Dillinger permissions"
                    Accessible.description: "Durably block context, display, suggestions, learning, and retention."
                  }

                  Item { Layout.fillWidth: true }

                  Button {
                    text: "Allow bundle"
                    iconText: "󰌿"
                    focusable: true
                    bordered: true
                    enabled: client.canGrantSubjects
                      && !client.targetBundleAllowed
                      && !client.targetCapacityReached
                    foreground: root.foreground
                    accent: root.accent
                    onClicked: client.allowTarget()
                    Accessible.name: "Allow Dillinger suggestion bundle"
                    Accessible.description: "Requires a healthy aggregate store and coherent settings."
                  }
                }
              }
            }

            BorderSurface {
              Layout.fillWidth: true
              implicitHeight: memoryContent.implicitHeight + Style.space(24)
              color: Style.normalFillFor(root.foreground, root.accent)
              radius: Style.cornerRadius
              borderSpec: Border.controlSpec("normal", root.foreground, root.accent)

              ColumnLayout {
                id: memoryContent
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.margins: Style.space(12)
                spacing: Style.space(8)

                PanelSectionHeader {
                  text: "TEXT-FREE OUTCOME AGGREGATES"
                  foreground: root.foreground
                }

                RowLayout {
                  Layout.fillWidth: true

                  Text {
                    textFormat: Text.PlainText
                    text: "Store"
                    color: root.foreground
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                  }

                  Item { Layout.fillWidth: true }

                  Text {
                    textFormat: Text.PlainText
                    text: !client.memoryStoreAvailableReported ? "Not reported"
                      : client.memoryStoreAvailable
                        ? client.memoryIntegrity : "Unavailable · preserved"
                    color: client.memoryStoreAvailable ? root.foreground : root.urgent
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                    font.bold: true
                  }
                }

                Text {
                  Layout.fillWidth: true
                  textFormat: Text.PlainText
                  text: (client.memoryRecords >= 0 ? client.memoryRecords : "Not reported")
                    + " records · " + root.byteLabel(client.memoryBytes)
                    + (client.targetLearningAllowed ? " · collection allowed" : " · collection blocked")
                  color: Qt.darker(root.foreground, 1.4)
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  wrapMode: Text.WordWrap
                }

                RowLayout {
                  Layout.fillWidth: true
                  spacing: Style.space(8)

                  Text {
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: root.confirmMemoryClear
                      ? "Confirm deletion of all aggregate counters."
                      : "Explicit clear is the only repair for unavailable aggregate bytes."
                    color: root.confirmMemoryClear ? root.urgent
                      : Qt.darker(root.foreground, 1.4)
                    font.family: Style.font.family
                    font.pixelSize: Style.font.caption
                    wrapMode: Text.WordWrap
                  }

                  Button {
                    text: root.confirmMemoryClear ? "Confirm clear" : "Clear Memory"
                    focusable: true
                    bordered: true
                    enabled: !client.busy
                      && client.brokerReachable
                      && client.memoryCommandAvailableReported
                      && client.memoryCommandAvailable
                    foreground: root.urgent
                    onClicked: {
                      if (!root.confirmMemoryClear) {
                        root.confirmMemoryClear = true
                        return
                      }
                      root.confirmMemoryClear = false
                      client.clearMemory()
                    }
                    Accessible.name: root.confirmMemoryClear
                      ? "Confirm clear Memory" : "Clear Memory"
                  }
                }
              }
            }

            BorderSurface {
              Layout.fillWidth: true
              visible: client.message !== ""
              implicitHeight: messageText.implicitHeight + Style.space(20)
              color: client.messageTone === "danger"
                ? Style.hoverFillFor(root.urgent, root.urgent)
                : Style.normalFillFor(root.foreground, root.accent)
              radius: Style.cornerRadius
              borderSpec: Border.controlSpec(
                "normal",
                client.messageTone === "danger" ? root.urgent : root.foreground,
                root.accent)

              Text {
                id: messageText
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.margins: Style.space(10)
                textFormat: Text.PlainText
                text: client.message
                color: client.messageTone === "danger" ? root.urgent : root.foreground
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                wrapMode: Text.WordWrap
                Accessible.role: Accessible.StaticText
                Accessible.name: client.message
              }
            }

            Item {
              Layout.fillWidth: true
              implicitHeight: Style.space(4)
            }
          }
        }
      }
    }
  }

  Component.onDestruction: client.dispose()
}
