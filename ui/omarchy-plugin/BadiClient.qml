import QtQuick
import Quickshell
import Quickshell.Io

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

  function isBrowserIdentity(value) {
    return hasExactKeys(value, ["kind", "adapter", "scheme", "host", "port"])
      && value.kind === "browser_origin"
      && value.adapter === "chromium"
      && (value.scheme === "http" || value.scheme === "https")
      && typeof value.host === "string"
      && value.host.length > 0
      && value.host.length <= 253
      && /^[a-z0-9.:-]+$/.test(value.host)
      && Number.isSafeInteger(value.port)
      && value.port >= 1
      && value.port <= 65535
  }

  function isLinuxIdentity(value) {
    return hasExactKeys(value, ["kind", "adapter", "app_id"])
      && value.kind === "linux_app"
      && (value.adapter === "fcitx" || value.adapter === "obsidian" || value.adapter === "shell"
        || value.adapter === "neovim" || value.adapter === "libreoffice")
      && typeof value.app_id === "string"
      && /^[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)*$/.test(value.app_id)
      && value.app_id.length <= 128
  }

  function isIdentity(value) {
    return isBrowserIdentity(value) || isLinuxIdentity(value)
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
      && (value.identity.kind !== "linux_app" || value.permissions.learn === "block")
  }

  function compareIdentities(left, right) {
    var kindOrder = {"browser_origin": 0, "linux_app": 1}
    var leftKind = kindOrder[left.kind]
    var rightKind = kindOrder[right.kind]
    if (leftKind !== rightKind) return leftKind - rightKind
    var fields = left.kind === "browser_origin"
      ? ["adapter", "scheme", "host"]
      : ["adapter", "app_id"]
    for (var index = 0; index < fields.length; index += 1) {
      var field = fields[index]
      if (left[field] < right[field]) return -1
      if (left[field] > right[field]) return 1
    }
    return left.kind === "browser_origin" ? left.port - right.port : 0
  }

  // Codes in `order` without repeats, in that order.
  function isOrderedCodes(value, order) {
    if (!Array.isArray(value)) return false
    for (var index = 0; index < value.length; index += 1) {
      var position = order.indexOf(value[index])
      if (position < 0 || (index > 0 && position <= order.indexOf(value[index - 1]))) return false
    }
    return true
  }

  // Whether `value` is one the settings schema
  // (broker/schemas/badi.settings.v2.schema.json) allows for the optional
  // key `key`. Producers omit a key at its default. A key this panel does
  // not know fails the whole document, so the panel never writes over a
  // setting it cannot see; run-client-lifecycle.sh keeps this list equal to
  // the schema's.
  function isOptionalSetting(key, value) {
    var writingCodes = ["en", "de", "fa", "fr", "es", "pt"]
    switch (key) {
    case "all_web_origins":
    case "all_linux_apps":
    case "mid_line":
    case "style_learning":
    case "context_words":
    case "languages_chosen":
      return typeof value === "boolean"
    case "escape_quiet_seconds":
      return Number.isSafeInteger(value) && value >= 1 && value <= 3600
    case "battery_mode":
      return ["normal", "quiet", "explicit"].indexOf(value) >= 0
    case "screen_share":
      return value === "hold" || value === "suggest"
    case "suggestion_length":
      return value === "short" || value === "normal"
    case "spelling_languages":
      return isOrderedCodes(value, ["en", "de", "fr", "es", "it", "nl", "pl"])
    case "writing_languages":
      return isOrderedCodes(value, writingCodes)
    case "trailing_space_healing":
      return isObject(value) && Object.keys(value).every(function(code) {
        return writingCodes.indexOf(code) >= 0 && typeof value[code] === "boolean"
      })
    case "writing_model":
      return typeof value === "string" && /^[a-z0-9.-]{1,64}$/.test(value)
    case "context_words_off":
      // In the broker's identity order.
      if (!Array.isArray(value) || value.length === 0 || value.length > 64) return false
      for (var index = 0; index < value.length; index += 1) {
        if (!isIdentity(value[index])
            || (index > 0 && compareIdentities(value[index - 1], value[index]) >= 0)) return false
      }
      return true
    case "language_overrides":
      return isObject(value) && Object.keys(value).length <= 64 && Object.keys(value).every(function(app) {
        return app.length <= 128 && /^[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)*$/.test(app)
          && writingCodes.indexOf(value[app]) >= 0
      })
    default:
      return false
    }
  }

  function isSettingsDocument(value) {
    if (!isObject(value)) return false
    var required = ["schema", "revision", "paused", "subjects"]
    var keys = Object.keys(value)
    for (var key = 0; key < keys.length; key += 1) {
      if (required.indexOf(keys[key]) < 0 && !isOptionalSetting(keys[key], value[keys[key]])) return false
    }
    if (value.schema !== "badi.settings.v2"
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
