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

    function deactivate(): void {
      client.deactivate()
    }

    function state(): string {
      return JSON.stringify({
        active: client.active,
        busy: client.busy,
        lifecycleGeneration: client.lifecycleGeneration,
        refreshQueued: client.refreshQueued
      })
    }
  }
}
