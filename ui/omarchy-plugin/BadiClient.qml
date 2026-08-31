import QtQuick
import Quickshell
import Quickshell.Io

Scope {
  id: root

  property var overview: ({})
  property string message: ""
  property string messageTone: "neutral"
  property bool overviewTimedOut: false
  property bool mutationTimedOut: false
  property string pendingSuccessMessage: ""
  property bool preserveMessageOnRefresh: false
  property bool active: false
  property bool disposed: false
  property int lifecycleGeneration: 0
  property int overviewGeneration: -1
  property int mutationGeneration: -1
  property bool refreshQueued: false
  property bool queuedPreserveMessage: false

  readonly property bool loading: overviewProcess.running
  readonly property bool mutating: mutationProcess.running
  readonly property bool busy: loading || mutating

  readonly property var broker: objectValue(overview, "broker")
  readonly property var settings: objectValue(overview, "settings")
  readonly property var privacy: objectValue(overview, "privacy")

  readonly property bool brokerReachable: boolValue(broker, "reachable", false)
  readonly property bool brokerPaused: boolValue(broker, "paused", true)
  readonly property bool brokerPausedReported: hasBoolean(broker, "paused")
  readonly property double brokerSettingsRevision: integerValue(broker, "settings_revision", -1)
  readonly property double authorityEpoch: integerValue(broker, "authority_epoch", -1)
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
    && !busy

  readonly property bool memoryStoreAvailable: boolValue(
    privacy, "memory_store_available", false)
  readonly property bool memoryStoreAvailableReported: hasBoolean(
    privacy, "memory_store_available")
  readonly property string memoryIntegrity: stringValue(
    privacy, "memory_integrity", "unknown")
  readonly property bool memoryCommandAvailable: boolValue(
    privacy, "memory_command_available", false)
  readonly property bool memoryCommandAvailableReported: hasBoolean(
    privacy, "memory_command_available")
  readonly property double memoryRecords: integerValue(privacy, "memory_records", -1)
  readonly property double memoryBytes: integerValue(privacy, "memory_bytes", -1)

  // Aggregate corruption must never remove the primary revoke control. Grants
  // stay gated on a healthy store because they can reinterpret retained state.
  readonly property bool canRevokeSubjects: canMutateSettings
  readonly property bool canGrantSubjects: canMutateSettings
    && memoryStoreAvailableReported
    && memoryStoreAvailable

  readonly property int targetSubjectIndex: settingsDocumentValid
    ? findTargetSubject(settings.subjects) : -1
  readonly property var targetSubject: targetSubjectIndex >= 0
    ? settings.subjects[targetSubjectIndex] : ({})
  readonly property var targetPermissions: objectValue(targetSubject, "permissions")
  readonly property bool targetBundleAllowed: targetSubjectIndex >= 0
    && targetPermissions.context_read === "allow"
    && targetPermissions.display === "allow"
    && targetPermissions.suggest === "allow"
  readonly property bool targetAnyAuthority: targetSubjectIndex >= 0
    && (targetPermissions.context_read === "allow"
      || targetPermissions.display === "allow"
      || targetPermissions.suggest === "allow"
      || targetPermissions.learn === "allow"
      || isBoundedRetention(targetPermissions.retention))
  readonly property bool targetCapacityReached: settingsDocumentValid
    && targetSubjectIndex < 0
    && settings.subjects.length >= 64
  readonly property bool targetLearningAllowed: targetSubjectIndex >= 0
    && targetPermissions.learn === "allow"

  signal refreshed()

  function isObject(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value)
  }

  function objectValue(parent, key) {
    return isObject(parent) && isObject(parent[key]) ? parent[key] : ({})
  }

  function hasBoolean(parent, key) {
    return isObject(parent) && typeof parent[key] === "boolean"
  }

  function boolValue(parent, key, fallback) {
    return hasBoolean(parent, key) ? parent[key] : fallback
  }

  function stringValue(parent, key, fallback) {
    return isObject(parent) && typeof parent[key] === "string" && parent[key].length > 0
      ? parent[key] : fallback
  }

  function integerValue(parent, key, fallback) {
    return isObject(parent) && Number.isSafeInteger(parent[key]) ? parent[key] : fallback
  }

  function hasExactKeys(value, keys) {
    if (!isObject(value)) return false
    var actual = Object.keys(value).sort()
    var expected = keys.slice().sort()
    if (actual.length !== expected.length) return false
    for (var index = 0; index < expected.length; index += 1) {
      if (actual[index] !== expected[index]) return false
    }
    return true
  }

  function isDecision(value) {
    return value === "allow" || value === "block"
  }

  function isRetention(value) {
    if (!isObject(value) || typeof value.mode !== "string") return false
    if (value.mode === "none") return hasExactKeys(value, ["mode"])
    return value.mode === "bounded"
      && hasExactKeys(value, ["mode", "days"])
      && Number.isSafeInteger(value.days)
      && value.days >= 1
      && value.days <= 90
  }

  function isBoundedRetention(value) {
    return isRetention(value) && value.mode === "bounded"
  }

  function isIdentity(value) {
    return hasExactKeys(value, ["kind", "adapter", "scheme", "host", "port"])
      && value.kind === "browser_origin"
      && value.adapter === "chromium"
      && (value.scheme === "http" || value.scheme === "https")
      && typeof value.host === "string"
      && value.host.length > 0
      && Number.isSafeInteger(value.port)
      && value.port >= 1
      && value.port <= 65535
  }

  function isPermissions(value) {
    if (!hasExactKeys(value,
        ["suggest", "display", "context_read", "learn", "retention"])) return false
    if (!isDecision(value.suggest)
        || !isDecision(value.display)
        || !isDecision(value.context_read)
        || !isDecision(value.learn)
        || !isRetention(value.retention)) return false
    if (value.suggest === "allow"
        && (value.display !== "allow" || value.context_read !== "allow")) return false
    if (value.learn === "allow"
        && (value.suggest !== "allow"
          || value.display !== "allow"
          || value.context_read !== "allow")) return false
    return value.retention.mode !== "bounded" || value.learn === "allow"
  }

  function isSubject(value) {
    return hasExactKeys(value, ["identity", "permissions"])
      && isIdentity(value.identity)
      && isPermissions(value.permissions)
  }

  function compareIdentities(left, right) {
    var fields = ["kind", "adapter", "scheme", "host"]
    for (var index = 0; index < fields.length; index += 1) {
      var field = fields[index]
      if (left[field] < right[field]) return -1
      if (left[field] > right[field]) return 1
    }
    return left.port - right.port
  }

  function isSettingsDocument(value) {
    if (!hasExactKeys(value, ["schema", "revision", "paused", "subjects"])
        || value.schema !== "badi.settings.v1"
        || !Number.isSafeInteger(value.revision)
        || value.revision < 0
        || typeof value.paused !== "boolean"
        || !Array.isArray(value.subjects)
        || value.subjects.length > 64) return false
    for (var index = 0; index < value.subjects.length; index += 1) {
      if (!isSubject(value.subjects[index])) return false
      if (index > 0 && compareIdentities(
          value.subjects[index - 1].identity,
          value.subjects[index].identity) >= 0) return false
    }
    return true
  }

  function isTargetIdentity(identity) {
    return isIdentity(identity)
      && identity.scheme === "https"
      && identity.host === "dillinger.io"
      && identity.port === 443
  }

  function findTargetSubject(subjects) {
    if (!Array.isArray(subjects)) return -1
    for (var index = 0; index < subjects.length; index += 1) {
      if (isSubject(subjects[index]) && isTargetIdentity(subjects[index].identity))
        return index
    }
    return -1
  }

  function cloneSettings() {
    return settingsDocumentValid ? JSON.parse(JSON.stringify(settings)) : null
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
    if (!preserveMessageOnRefresh) message = ""
    overviewTimedOut = false
    overviewGeneration = lifecycleGeneration
    overviewProcess.exec(["badictl", "overview", "--json"])
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
      messageTone = "danger"
      message = "Refresh a coherent settings document before changing policy."
      return
    }
    if (settingsRevision >= Number.MAX_SAFE_INTEGER) {
      messageTone = "danger"
      message = "The settings revision cannot be advanced safely."
      return
    }
    document.revision = settingsRevision + 1
    if (!isSettingsDocument(document)) {
      messageTone = "danger"
      message = "The requested change did not produce valid badi.settings.v1 data."
      return
    }
    pendingSuccessMessage = successMessage
    message = ""
    mutationTimedOut = false
    mutationGeneration = lifecycleGeneration
    mutationProcess.exec([
      "badictl", "settings", "replace",
      "--if-revision", String(settingsRevision),
      "--json", JSON.stringify(document)
    ])
  }

  function blockTarget() {
    if (!canRevokeSubjects) {
      messageTone = "danger"
      message = "Refresh a coherent settings document before blocking Dillinger."
      return
    }
    var document = cloneSettings()
    var index = document === null ? -1 : findTargetSubject(document.subjects)
    if (index < 0 || !targetAnyAuthority) return
    document.subjects[index].permissions = {
      "suggest": "block",
      "display": "block",
      "context_read": "block",
      "learn": "block",
      "retention": { "mode": "none" }
    }
    replaceSettings(document, "https://dillinger.io is durably blocked.")
  }

  function allowTarget() {
    if (!canGrantSubjects) {
      messageTone = "danger"
      message = "Clear or repair Memory before granting Dillinger authority."
      return
    }
    var document = cloneSettings()
    if (document === null) return
    var index = findTargetSubject(document.subjects)
    if (index < 0) {
      if (document.subjects.length >= 64) {
        messageTone = "warning"
        message = "The 64-subject settings limit is reached."
        return
      }
      document.subjects.push({
        "identity": {
          "kind": "browser_origin",
          "adapter": "chromium",
          "scheme": "https",
          "host": "dillinger.io",
          "port": 443
        },
        "permissions": {
          "suggest": "block",
          "display": "block",
          "context_read": "block",
          "learn": "block",
          "retention": { "mode": "none" }
        }
      })
      document.subjects.sort(function(left, right) {
        return compareIdentities(left.identity, right.identity)
      })
      index = findTargetSubject(document.subjects)
    }
    document.subjects[index].permissions.context_read = "allow"
    document.subjects[index].permissions.display = "allow"
    document.subjects[index].permissions.suggest = "allow"
    replaceSettings(document, "The Dillinger suggestion bundle is allowed.")
  }

  function clearMemory() {
    if (!active || disposed || busy || !brokerReachable || !memoryCommandAvailableReported
        || !memoryCommandAvailable) {
      messageTone = "danger"
      message = "The broker's explicit Memory clear command is unavailable."
      return
    }
    pendingSuccessMessage = "Text-free outcome aggregates were cleared."
    message = ""
    mutationTimedOut = false
    mutationGeneration = lifecycleGeneration
    mutationProcess.exec(["badictl", "memory", "clear"])
  }

  function deactivate() {
    if (disposed) return
    active = false
    lifecycleGeneration += 1
    refreshQueued = false
    queuedPreserveMessage = false
    pendingSuccessMessage = ""
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

  function dispose() {
    if (disposed) return
    disposed = true
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
          || exitedGeneration !== root.lifecycleGeneration) {
        root.scheduleQueuedRefresh()
        return
      }
      if (root.overviewTimedOut) {
        root.overview = ({})
        root.messageTone = "danger"
        root.message = "Reading Badi status exceeded five seconds."
        root.preserveMessageOnRefresh = false
        root.refreshed()
        root.scheduleQueuedRefresh()
        return
      }
      if (exitCode !== 0) {
        root.overview = ({})
        root.messageTone = "danger"
        root.message = root.safeError(
          overviewStderr.text,
          "Could not read Badi status. Is the broker running?")
        root.preserveMessageOnRefresh = false
        root.refreshed()
        root.scheduleQueuedRefresh()
        return
      }
      try {
        var parsed = JSON.parse(overviewStdout.text)
        if (!root.isObject(parsed) || parsed.schema !== "badi.overview.v1")
          throw new Error("unexpected overview schema")
        root.overview = parsed
        if (!root.preserveMessageOnRefresh) root.message = ""
      } catch (error) {
        root.overview = ({})
        root.messageTone = "danger"
        root.message = "badictl returned invalid badi.overview.v1 data."
      }
      root.preserveMessageOnRefresh = false
      root.refreshed()
      root.scheduleQueuedRefresh()
    }
  }

  Process {
    id: mutationProcess
    stdout: StdioCollector {}
    stderr: StdioCollector { id: mutationStderr }

    onStarted: mutationTimeout.restart()
    onExited: function(exitCode) {
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
      root.messageTone = succeeded ? "positive" : "danger"
      root.message = root.mutationTimedOut
        ? "The Badi change exceeded five seconds; refreshing status."
        : succeeded
          ? root.pendingSuccessMessage
          : root.safeError(mutationStderr.text, "Badi rejected the requested change.")
      root.pendingSuccessMessage = ""
      root.queueRefresh(true)
      root.scheduleQueuedRefresh()
    }
  }

  Component.onDestruction: root.dispose()
}
