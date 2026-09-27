import QtQuick
import Quickshell
import Quickshell.Io

ShellRoot {
  id: root

  BadiClient {
    id: client
  }

  IpcHandler {
    target: "badi-client-lifecycle"

    function ping(): string {
      return "ok"
    }

    function activate(): void {
      client.activate()
    }

    function refresh(): void { client.refresh(true) }

    // The same compare-and-swap settings write the desktop panel's pause and
    // app toggles perform.
    function setPaused(paused: bool): void {
      var document = client.cloneSettings()
      if (document !== null) document.paused = paused
      client.replaceSettings(document, paused ? "Predictions paused." : "Predictions resumed.")
    }

    function deactivate(): void {
      client.deactivate()
    }

    function validateSettings(documentJson: string): string {
      try {
        return client.isSettingsDocument(JSON.parse(documentJson)) ? "true" : "false"
      } catch (error) {
        return "false"
      }
    }

    function state(): string {
      return JSON.stringify({
        active: client.active,
        busy: client.busy,
        loading: client.loading,
        canMutateSettings: client.canMutateSettings,
        message: client.message,
        lifecycleGeneration: client.lifecycleGeneration,
        refreshQueued: client.refreshQueued,
        overviewSchema: client.overview.schema || "",
        settingsSchema: client.settings.schema || "",
        settingsDocumentValid: client.settingsDocumentValid,
        subjectCount: client.settings.subjects ? client.settings.subjects.length : -1
      })
    }
  }
}
