import QtQuick
import Quickshell
import Quickshell.Io
import "KeyModel.js" as KeyModel

// The real DesktopPanel (state and actions, no visuals) offscreen, so its
// settings changes run against the fake badi-desktop and badictl.
ShellRoot {
  id: harnessRoot
  property var snapshotRows: []
  property int cursor: -1
  property string cursorKey: ""

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

    // Every row (All settings, or a first-run question), as key, section,
    // kind, label, detail, value and checked.
    function rows(): string {
      panel.showAll = true
      return JSON.stringify(panel.rows().map(function(row) {
        return {key: row.key, section: row.section, kind: row.kind, label: row.label, detail: row.detail,
          value: row.value === undefined ? null : row.value, checked: row.checked === true,
          options: row.options || [], enabled: row.enabled === true}
      }))
    }

    function captureRows(cursorKey: string): string {
      panel.showAll = true
      harnessRoot.snapshotRows = panel.rows()
      harnessRoot.cursor = KeyModel.follow(harnessRoot.snapshotRows, cursorKey, 0)
      harnessRoot.cursorKey = harnessRoot.cursor >= 0 && harnessRoot.cursor < harnessRoot.snapshotRows.length
        ? harnessRoot.snapshotRows[harnessRoot.cursor].key : ""
      return JSON.stringify({cursor: harnessRoot.cursor, cursorKey: harnessRoot.cursorKey, count: harnessRoot.snapshotRows.length})
    }

    function checkPollSurvival(): string {
      panel.showAll = true
      var latest = panel.rows()
      var same = KeyModel.sameRows(harnessRoot.snapshotRows, latest)
      if (!same) harnessRoot.snapshotRows = latest
      harnessRoot.cursor = KeyModel.follow(harnessRoot.snapshotRows, harnessRoot.cursorKey, harnessRoot.cursor)
      harnessRoot.cursorKey = harnessRoot.cursor >= 0 && harnessRoot.cursor < harnessRoot.snapshotRows.length
        ? harnessRoot.snapshotRows[harnessRoot.cursor].key : ""
      return JSON.stringify({same: same, cursor: harnessRoot.cursor, cursorKey: harnessRoot.cursorKey, count: harnessRoot.snapshotRows.length})
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
        ready: panel.ready,
        paused: panel.paused,
        loading: panel.loading,
        statusLabel: panel.statusLabel,
        mode: panel.mode,
        settingsRevision: panel.client.settingsRevision,
        settingsDocumentValid: panel.client.settingsDocumentValid,
        message: panel.client.message,
        messageTone: panel.client.messageTone,
        actionMessage: panel.actionMessage,
        actionFailed: panel.actionFailed,
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
