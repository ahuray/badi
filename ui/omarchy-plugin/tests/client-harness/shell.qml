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

    function clearMemory(): void {
      client.clearMemory()
    }

    function blockTarget(): void {
      client.blockTarget()
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
        lifecycleGeneration: client.lifecycleGeneration,
        refreshQueued: client.refreshQueued,
        overviewSchema: client.overview.schema || "",
        supportScope: client.overview.support ? client.overview.support.scope || "" : "",
        supportGeneralization: client.overview.support
          ? client.overview.support.generalization || "" : "",
        supportAuthorization: client.overview.support
          ? client.overview.support.authorization || "" : "",
        verifiedSupportCells: client.overview.support
          && client.overview.support.verified_cells
          ? client.overview.support.verified_cells.length : -1,
        browserSupportActivation: client.overview.support
          && client.overview.support.verified_cells
          ? client.overview.support.verified_cells[0].required_activation || "" : "",
        nativeSupportActivation: client.overview.support
          && client.overview.support.verified_cells
          ? client.overview.support.verified_cells[1].required_activation || "" : "",
        settingsSchema: client.settings.schema || "",
        settingsDocumentValid: client.settingsDocumentValid,
        subjectCount: client.settings.subjects ? client.settings.subjects.length : -1,
        targetSubjectIndex: client.targetSubjectIndex
      })
    }
  }
}
