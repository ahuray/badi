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

    // Every row (All settings, or a first-run question), as key, label,
    // detail, value and checked.
    function rows(): string {
      panel.showAll = true
      return JSON.stringify(panel.rows().map(function(row) {
        return {key: row.key, label: row.label, detail: row.detail,
          value: row.value === undefined ? null : row.value, checked: row.checked === true,
          options: row.options || [], enabled: row.enabled === true}
      }))
    }

    // A row's choice or action, as a click runs it.
    function chooseRow(key: string, value: string): void {
      panel.rows().find(function(row) { return row.key === key }).choose(value)
    }

    function activateRow(key: string): void {
      var row = panel.rows().find(function(row) { return row.key === key })
      if (row.enabled) row.activate()
    }

    function confirm(accepted: bool): void { panel.resolveConfirmation(accepted) }

    function state(): string {
      return JSON.stringify({
        busy: panel.controlsBusy,
        canChange: panel.canChange,
        settingsDocumentValid: panel.client.settingsDocumentValid,
        message: panel.client.message,
        actionMessage: panel.actionMessage,
        confirming: panel.confirming,
        statsReported: panel.statsReported,
        statsAvailable: panel.statsAvailable,
        statsWeek: panel.statsWeek(),
        statsToday: panel.statsTotals("today"),
        statsLeaders: panel.statsLeaders(3),
        chosenLanguages: panel.chosenLanguages()
      })
    }
  }
}
