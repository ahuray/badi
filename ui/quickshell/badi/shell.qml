//@ pragma ShellId badi-control-center
//@ pragma AppId io.github.ahuray.badi.control-center

import QtQuick
import Quickshell
import Quickshell.Io

ShellRoot {
    id: root

    BadiClient {
        id: badiClient
    }

    FloatingWindow {
        id: controlWindow

        title: "Badi Control Center"
        visible: true
        implicitWidth: 760
        implicitHeight: 820
        minimumSize: Qt.size(580, 640)
        maximumSize: Qt.size(980, 1100)
        color: BadiTheme.window

        ControlCenter {
            id: controlCenter
            anchors.fill: parent
            client: badiClient
            onCloseRequested: controlWindow.visible = false
        }

        onVisibleChanged: {
            if (visible) {
                badiClient.refresh();
                Qt.callLater(() => controlCenter.forceActiveFocus());
            } else {
                controlCenter.resetTransientState();
            }
        }
        onClosed: {
            controlCenter.resetTransientState();
            visible = false;
        }
        Component.onCompleted: controlCenter.forceActiveFocus()
    }

    IpcHandler {
        target: "badi"

        function show(): void {
            controlWindow.visible = true;
            badiClient.refresh();
        }

        function hide(): void {
            controlWindow.visible = false;
        }

        function toggle(): void {
            controlWindow.visible = !controlWindow.visible;
            if (controlWindow.visible) badiClient.refresh();
        }

        function refresh(): void {
            badiClient.refresh();
        }
    }
}
