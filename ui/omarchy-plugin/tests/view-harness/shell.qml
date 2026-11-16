import QtQuick
import QtQuick.Window
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import qs.Ui

// The real SettingsView inside Omarchy's PanelKeyCatcher in an offscreen
// window, exercising inline site-field focus, popout close/reopen reset and
// delegate destruction against the actual QML focus chain.
ShellRoot {
  id: harnessRoot

  DesktopPanel {
    id: badi
  }

  TextInput {
    id: replacementInput
    visible: false
  }

  QtObject {
    id: fakeBar
    readonly property color foreground: Commons.Color.foreground
    readonly property string fontFamily: Commons.Style.font.family
  }

  Window {
    id: popout
    visible: true
    width: 420
    height: 640

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      blocked: view.editing
      onMoveRequested: function(dx, dy) { view.move(dx, dy) }
      onActivateRequested: view.activate()
      onDeleteRequested: view.remove()
      onTextKey: function(text) {
        if (text === "/") view.focusSearch()
        else if (text === "a" || text === "A") badi.showAll = !badi.showAll
      }

      SettingsView {
        id: view
        anchors.fill: parent
        controller: badi
        bar: fakeBar
        onEditingFinished: keyCatcher.forceActiveFocus()
      }
    }
  }

  IpcHandler {
    target: "badi-view-lifecycle"

    function ping(): string {
      return "ok"
    }

    function openPopout(): void {
      popout.requestActivate()
      view.reset()
      badi.refresh()
      keyCatcher.forceActiveFocus()
    }

    function focusSiteField(draft: string): string {
      popout.requestActivate()
      badi.showAll = true
      for (var i = 0; i < view.rows.length + 2; i++) {
        if (view.cursorKey === "site_add") break
        view.move(0, 1)
      }
      view.activate()
      if (view.siteInput && draft !== "") view.siteInput.text = draft
      return state()
    }

    function reopenWithoutEscape(): string {
      // Mirrors BarWidget.onOpenedChanged + KeyboardPanel.focusTarget when the
      // popout closes while siteField holds focus and is opened again.
      popout.requestActivate()
      view.reset()
      badi.refresh()
      keyCatcher.forceActiveFocus()
      return state()
    }

    function collapseWithoutReset(): void {
      badi.showAll = false
      keyCatcher.forceActiveFocus()
    }

    function supersedeAndCollapse(): void {
      view.siteInput = replacementInput
      view.siteInputFocused = true
      badi.showAll = false
    }

    function stepDown(): string {
      view.move(0, 1)
      return state()
    }

    function state(): string {
      return JSON.stringify({
        busy: badi.controlsBusy,
        windowActive: popout.active,
        showAll: badi.showAll,
        rowCount: view.rows.length,
        cursor: view.cursor,
        cursorKey: view.cursorKey,
        siteDraft: view.siteDraft,
        siteInputPresent: view.siteInput !== null,
        siteInputIsReplacement: view.siteInput === replacementInput,
        siteInputActiveFocus: view.siteInput !== null && view.siteInput.activeFocus === true,
        siteInputFocused: view.siteInputFocused,
        editing: view.editing,
        keyCatcherBlocked: keyCatcher.blocked,
        keyCatcherActiveFocus: keyCatcher.activeFocus
      })
    }
  }
}
