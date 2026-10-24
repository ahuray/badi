import QtQuick
import Quickshell
import Quickshell.Io

// The real DesktopPanel (state and actions, no visuals) offscreen, so its
// settings changes run against the fake badi-desktop and badictl.
ShellRoot {
  DesktopPanel {
    id: panel
  }

  IpcHandler {
    target: "badi-panel-settings"

    function ping(): string {
      return "ok"
    }

    function refresh(): void { panel.client.refresh(true) }

    function toggleLanguage(code: string): void { panel.toggleLanguage(code) }

    function setScreenShare(mode: string): void { panel.setScreenShare(mode) }

    // Every row (All settings), as key, label and value.
    function rows(): string {
      panel.showAll = true
      return JSON.stringify(panel.rows().map(function(row) {
        return {key: row.key, label: row.label, value: row.value === undefined ? null : row.value}
      }))
    }

    function state(): string {
      return JSON.stringify({
        busy: panel.client.busy,
        canChange: panel.canChange,
        settingsDocumentValid: panel.client.settingsDocumentValid,
        message: panel.client.message,
        chosenLanguages: panel.chosenLanguages()
      })
    }
  }
}
