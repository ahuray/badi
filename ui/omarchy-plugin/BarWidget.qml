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
  // This panel owns the badi-writing target to add state() and refresh()
  // beside toggle.
  manageIpc: false
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  DesktopPanel { id: badi }

  // ShellIpc also answers on the shell's own socket, so omarchy-shell
  // reaches it in milliseconds without a qs ipc client.
  ShellIpc {
    target: "badi-writing"
    function toggle(): void { root.toggle() }
    function open(): void { root.open() }
    function close(): void { root.close() }
    // The mark reads the broker only while the popout is open; a pause or
    // snooze from the terminal or a shortcut asks it to look now.
    function refresh(): void { badi.client.refresh(true) }
    function state(): string {
      return JSON.stringify({visible: root.opened, status: badi.statusLabel, page: view.cursor,
        paused: badi.paused, snoozed: badi.snoozeLabel, service: badi.serviceState,
        confirming: badi.confirming})
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
      // While the dialog asks, the keys answer it: h/l or arrows choose,
      // Enter decides, Escape cancels.
      onMoveRequested: function(dx, dy) {
        if (badi.confirming) { if (dx !== 0) confirm.selectedIndex = confirm.selectedIndex === 0 ? 1 : 0 }
        else view.move(dx, dy)
      }
      onActivateRequested: {
        if (badi.confirming) badi.resolveConfirmation(confirm.selectedIndex === 1)
        else view.activate()
      }
      onDeleteRequested: if (!badi.confirming) view.remove()
      onCloseRequested: {
        if (badi.confirming) badi.resolveConfirmation(false)
        else root.close()
      }
      onTabRequested: function(direction) {
        if (badi.confirming) confirm.selectedIndex = confirm.selectedIndex === 0 ? 1 : 0
        else root.switchPanel(direction)
      }
      onTextKey: function(text) {
        if (badi.confirming) return
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

      ConfirmDialog {
        id: confirm
        anchors.fill: parent
        z: 10
        opened: badi.confirming
        message: badi.confirming ? badi.confirmation.message : ""
        confirmText: badi.confirming ? badi.confirmation.label : ""
        cancelText: "Cancel"
        onOpenedChanged: if (opened) selectedIndex = 0
        onCanceled: badi.resolveConfirmation(false)
        onConfirmed: badi.resolveConfirmation(true)
      }
    }
  }
}
