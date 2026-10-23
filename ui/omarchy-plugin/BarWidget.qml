import QtQuick
import Quickshell
import Quickshell.Io
import qs.Ui
import qs.Commons
import qs.Commons as Commons

// The Badi bar mark and its settings popout, in Omarchy's own panel form:
// the popout hangs under the mark, takes the keyboard, and Tab moves on to
// the neighbouring bar panel.
Panel {
  id: root
  moduleName: "io.github.ahuray.badi"
  // This panel owns the badi-writing target to add state() beside toggle.
  manageIpc: false
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  DesktopPanel { id: badi }

  IpcHandler {
    target: "badi-writing"
    function toggle(): void { root.toggle() }
    function open(): void { root.open() }
    function close(): void { root.close() }
    function state(): string {
      return JSON.stringify({visible: root.opened, status: badi.statusLabel, page: view.cursor,
        paused: badi.paused, snoozed: badi.snoozeLabel, service: badi.serviceState})
    }
  }

  onOpenedChanged: {
    if (!opened) return
    view.reset()
    badi.refresh()
  }

  Timer {
    interval: 3000
    running: root.opened
    repeat: true
    onTriggered: { badi.client.refresh(true); badi.probeService() }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    iconComponent: BadiMark {
      ink: button.active ? button.activeColor : button.foreground
    }
    tooltipText: "Badi · " + badi.statusLabel + (badi.snoozeLabel ? "\nSnoozed in " + badi.snoozeLabel : "")
      + "\nClick: settings · Right-click: pause / resume"
    active: badi.ready
    activeColor: Commons.Color.accent
    dimmed: badi.paused
    onPressed: function(mouseButton) {
      if (mouseButton === Qt.RightButton) badi.togglePause()
      else root.toggle()
    }
    Accessible.name: "Badi writing settings"
    Accessible.role: Accessible.Button
    Accessible.onPressAction: root.toggle()
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(420))
    contentHeight: panel.fittedContentHeight(view.implicitHeight, Style.space(640))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      blocked: view.editing
      onMoveRequested: function(dx, dy) { view.move(dx, dy) }
      onActivateRequested: view.activate()
      onDeleteRequested: view.remove()
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(text) {
        if (text === "/") view.focusSearch()
        else if (text === "p" || text === "P") badi.togglePause()
        else if (text === "r" || text === "R") badi.refresh()
        else if (text === "a" || text === "A") badi.showAll = !badi.showAll
      }

      SettingsView {
        id: view
        anchors.fill: parent
        controller: badi
        bar: root.bar
        onEditingFinished: keyCatcher.forceActiveFocus()
        onCloseRequested: root.close()
      }
    }
  }
}
