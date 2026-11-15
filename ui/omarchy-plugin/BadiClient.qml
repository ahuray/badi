import QtQuick
import Quickshell
import Quickshell.Io
import "SettingsModel.js" as SettingsModel

Scope {
  id: root

  property var cliPrefix: ["badictl"]

  property var overview: ({})
  property string message: ""
  property string messageTone: "neutral"
  // The message reports a failed or refused read; the next good read clears
  // it, even on a refresh that otherwise keeps the last result.
  property bool messageFromRead: false
  property bool overviewTimedOut: false
  property bool overviewInvalidated: false
  property bool mutationTimedOut: false
  property string pendingSuccessMessage: ""
  property string pendingSettings: ""
  property bool preserveMessageOnRefresh: false
  property bool active: false
  property bool disposed: false
  property int lifecycleGeneration: 0
  property int overviewGeneration: -1
  property int mutationGeneration: -1
  property bool refreshQueued: false
  property bool queuedPreserveMessage: false
  // A settings document the validator refuses while the broker answers.
  readonly property string unknownSettingsMessage:
    "This panel does not know every Badi setting; update the panel. Changes stay off until then."

  readonly property bool loading: overviewProcess.running
  readonly property bool mutating: mutationProcess.running
  readonly property bool busy: loading || mutating

  readonly property var broker: objectValue(overview, "broker")
  readonly property var settings: objectValue(overview, "settings")

  readonly property bool brokerReachable: boolValue(broker, "reachable", false)
  readonly property bool brokerPaused: boolValue(broker, "paused", true)
  readonly property double brokerSettingsRevision: integerValue(broker, "settings_revision", -1)
  readonly property bool controlPlaneDegraded: boolValue(
    broker, "control_plane_degraded", true)
  readonly property bool controlPlaneDegradedReported: hasBoolean(
    broker, "control_plane_degraded")

  readonly property bool settingsDocumentValid: isSettingsDocument(settings)
  readonly property double settingsRevision: settingsDocumentValid ? settings.revision : -1
  readonly property bool snapshotCoherent: brokerSettingsRevision >= 0
    && brokerSettingsRevision === settingsRevision
  readonly property bool canMutateSettings: active
    && !disposed
    && brokerReachable
    && settingsDocumentValid
    && snapshotCoherent
    && controlPlaneDegradedReported
    && !controlPlaneDegraded
    && settingsRevision >= 0
    && !mutating

  function isObject(value) {
    return SettingsModel.isObject(value)
  }

  function objectValue(parent, key) {
    return SettingsModel.objectValue(parent, key)
  }

  function hasBoolean(parent, key) {
    return SettingsModel.hasBoolean(parent, key)
  }

  function boolValue(parent, key, fallback) {
    return SettingsModel.boolValue(parent, key, fallback)
  }

  function integerValue(parent, key, fallback) {
    return SettingsModel.integerValue(parent, key, fallback)
  }

  function isIdentity(value) {
    return SettingsModel.isIdentity(value)
  }

  function isTerminalIdentity(value) {
    return SettingsModel.isTerminalIdentity(value)
  }

  function compareIdentities(left, right) {
    return SettingsModel.compareIdentities(left, right)
  }

  function isSettingsDocument(value) {
    return SettingsModel.isSettingsDocument(value)
  }

  function cloneSettings() {
    return settingsDocumentValid ? JSON.parse(JSON.stringify(settings)) : null
  }

  function say(text, tone, fromRead) {
    message = text
    messageTone = tone
    messageFromRead = fromRead === true
  }

  function safeError(stderrText, fallback) {
    if (typeof stderrText !== "string") return fallback
    var firstLine = stderrText.trim().split("\n")[0]
    return firstLine.length > 0 ? firstLine.slice(0, 180) : fallback
  }

  function queueRefresh(preserveMessage) {
    if (disposed || !active) return
    if (!refreshQueued) queuedPreserveMessage = preserveMessage === true
    else queuedPreserveMessage = queuedPreserveMessage && preserveMessage === true
    refreshQueued = true
  }

  function scheduleQueuedRefresh() {
    if (disposed || !active || !refreshQueued) return
    refreshDrain.restart()
  }

  function refresh(preserveMessage) {
    if (disposed || !active) return
    if (busy) {
      queueRefresh(preserveMessage)
      return
    }
    refreshQueued = false
    queuedPreserveMessage = false
    preserveMessageOnRefresh = preserveMessage === true
    if (!preserveMessageOnRefresh) say("", "neutral")
    overviewTimedOut = false
    overviewInvalidated = false
    overviewGeneration = lifecycleGeneration
    overviewProcess.exec(cliPrefix.concat(["overview", "--json"]))
  }

  function activate() {
    if (disposed) return
    if (!active) {
      lifecycleGeneration += 1
      active = true
    }
    refresh(false)
  }

  function replaceSettings(document, successMessage) {
    if (!canMutateSettings || document === null) {
      say("Refresh a coherent settings document before changing policy.", "danger")
      return
    }
    if (settingsRevision >= Number.MAX_SAFE_INTEGER) {
      say("The settings revision cannot be advanced safely.", "danger")
      return
    }
    document.revision = settingsRevision + 1
    if (!isSettingsDocument(document)) {
      say("The requested change did not produce valid badi.settings.v2 data.", "danger")
      return
    }
    invalidateOverview()
    pendingSuccessMessage = successMessage
    say("", "neutral")
    mutationTimedOut = false
    mutationGeneration = lifecycleGeneration
    pendingSettings = JSON.stringify(document)
    mutationProcess.stdinEnabled = true
    mutationProcess.exec(cliPrefix.concat([
      "settings", "replace",
      "--if-revision", String(settingsRevision),
      "--json", "-"
    ]))
  }

  function deactivate() {
    if (disposed) return
    active = false
    lifecycleGeneration += 1
    refreshQueued = false
    queuedPreserveMessage = false
    pendingSuccessMessage = ""
    pendingSettings = ""
    preserveMessageOnRefresh = false
    refreshDrain.stop()
    overviewTimeout.stop()
    mutationTimeout.stop()
    if (overviewProcess.running) {
      overviewProcess.signal(15)
      overviewKillTimeout.restart()
    } else {
      overviewKillTimeout.stop()
    }
    if (mutationProcess.running) {
      mutationProcess.signal(15)
      mutationKillTimeout.restart()
    } else {
      mutationKillTimeout.stop()
    }
  }

  function invalidateOverview() {
    if (!overviewProcess.running) return
    // A read begun before the CAS write must never replace its new snapshot.
    overviewInvalidated = true
    overviewProcess.signal(15)
    overviewKillTimeout.restart()
  }

  function dispose() {
    if (disposed) return
    disposed = true
    pendingSettings = ""
    active = false
    lifecycleGeneration += 1
    refreshQueued = false
    queuedPreserveMessage = false
    refreshDrain.stop()
    overviewTimeout.stop()
    overviewKillTimeout.stop()
    mutationTimeout.stop()
    mutationKillTimeout.stop()
    if (overviewProcess.running) overviewProcess.signal(9)
    if (mutationProcess.running) mutationProcess.signal(9)
  }

  Timer {
    id: refreshDrain
    interval: 0
    repeat: false
    onTriggered: {
      if (root.disposed || !root.active || root.busy || !root.refreshQueued) return
      var preserveMessage = root.queuedPreserveMessage
      root.refreshQueued = false
      root.queuedPreserveMessage = false
      root.refresh(preserveMessage)
    }
  }

  Timer {
    id: overviewTimeout
    interval: 5000
    repeat: false
    onTriggered: {
      if (!overviewProcess.running) return
      root.overviewTimedOut = true
      overviewProcess.signal(15)
      overviewKillTimeout.restart()
    }
  }

  Timer {
    id: overviewKillTimeout
    interval: 500
    repeat: false
    onTriggered: if (overviewProcess.running) overviewProcess.signal(9)
  }

  Timer {
    id: mutationTimeout
    interval: 5000
    repeat: false
    onTriggered: {
      if (!mutationProcess.running) return
      root.mutationTimedOut = true
      mutationProcess.signal(15)
      mutationKillTimeout.restart()
    }
  }

  Timer {
    id: mutationKillTimeout
    interval: 500
    repeat: false
    onTriggered: if (mutationProcess.running) mutationProcess.signal(9)
  }

  Process {
    id: overviewProcess
    stdout: StdioCollector { id: overviewStdout }
    stderr: StdioCollector { id: overviewStderr }

    onStarted: overviewTimeout.restart()
    onExited: function(exitCode) {
      overviewTimeout.stop()
      overviewKillTimeout.stop()
      var exitedGeneration = root.overviewGeneration
      root.overviewGeneration = -1
      if (root.disposed || !root.active
          || root.overviewInvalidated || exitedGeneration !== root.lifecycleGeneration) {
        root.scheduleQueuedRefresh()
        return
      }
      if (root.overviewTimedOut) {
        root.overview = ({})
        root.say("Reading Badi status exceeded five seconds.", "danger", true)
        root.preserveMessageOnRefresh = false
        root.scheduleQueuedRefresh()
        return
      }
      if (exitCode !== 0) {
        root.overview = ({})
        root.say(root.safeError(overviewStderr.text, "Could not read Badi status. Is the broker running?"),
          "danger", true)
        root.preserveMessageOnRefresh = false
        root.scheduleQueuedRefresh()
        return
      }
      try {
        var parsed = JSON.parse(overviewStdout.text)
        if (!root.isObject(parsed) || parsed.schema !== "badi.overview.v2")
          throw new Error("unexpected overview schema")
        root.overview = parsed
        if (root.isObject(parsed.broker) && parsed.broker.reachable === true
            && !root.isSettingsDocument(parsed.settings)) {
          // Every write is then refused; say why rather than only dim the controls.
          root.say(root.unknownSettingsMessage, "danger", true)
        } else if (!root.preserveMessageOnRefresh || root.messageFromRead) root.say("", "neutral")
      } catch (error) {
        root.overview = ({})
        root.say("badictl returned invalid badi.overview.v2 data.", "danger", true)
      }
      root.preserveMessageOnRefresh = false
      root.scheduleQueuedRefresh()
    }
  }

  Process {
    id: mutationProcess
    stdout: StdioCollector {}
    stderr: StdioCollector { id: mutationStderr }

    onStarted: {
      mutationTimeout.restart()
      if (root.active && !root.disposed && root.mutationGeneration === root.lifecycleGeneration)
        write(root.pendingSettings)
      root.pendingSettings = ""
      stdinEnabled = false
    }
    onExited: function(exitCode) {
      root.pendingSettings = ""
      mutationTimeout.stop()
      mutationKillTimeout.stop()
      var exitedGeneration = root.mutationGeneration
      root.mutationGeneration = -1
      if (root.disposed || !root.active
          || exitedGeneration !== root.lifecycleGeneration) {
        root.scheduleQueuedRefresh()
        return
      }
      var succeeded = exitCode === 0 && !root.mutationTimedOut
      root.say(root.mutationTimedOut
        ? "The Badi change exceeded five seconds; refreshing status."
        : succeeded
          ? root.pendingSuccessMessage
          : root.safeError(mutationStderr.text, "Badi rejected the requested change."),
        succeeded ? "positive" : "danger")
      root.pendingSuccessMessage = ""
      root.queueRefresh(true)
      root.scheduleQueuedRefresh()
    }
  }

  Component.onDestruction: root.dispose()
}
