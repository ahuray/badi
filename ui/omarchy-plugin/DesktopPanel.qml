import QtQuick
import Quickshell
import Quickshell.Io

// The panel's state and actions, without any visuals: the broker client,
// the desktop helper's processes, every settings change (compare-and-swap
// through BadiClient) and the typed rows SettingsView renders. Rows are
// plain descriptors, so search and keyboard navigation work on one list.
Item {
  id: root

  readonly property string helper: Quickshell.env("HOME") + "/.local/bin/badi-desktop"
  // Supported apps; `manual` apps need an exact rule even in blocklist mode.
  readonly property var supportedApps: [
    {name: "Omawrite", appId: "omawrite", detail: "Markdown editor"},
    {name: "Ghostty", appId: "com.mitchellh.ghostty", detail: "Terminal · follows the line you type", manual: true},
    {name: "Xournal++", appId: "com.github.xournalpp.xournalpp", detail: "Text tool · Tab requests, Tab again accepts a word", manual: true},
    {name: "Obsidian", appId: "obsidian", detail: "Badi plugin · Tab a word, Ctrl+Right all"},
    {name: "Neovim", appId: "nvim", detail: "Badi plugin · prose buffers"},
    {name: "Bash", appId: "bash", detail: "Ctrl-X Tab requests and accepts"},
    {name: "LibreOffice Writer", appId: "libreoffice", detail: "Grey preview in paragraphs"},
    {name: "Writer spelling", appId: "libreoffice-writer", detail: "Corrections after a word · Alt+F7"},
    {name: "Telegram", appId: "telegram", detail: "Message fields"},
    {name: "Codex", appId: "chatgpt", detail: "Composer"},
    {name: "VS Code", appId: "code", detail: "Needs \"editor.editContext\": false"},
    {name: "Cursor", appId: "cursor", detail: "Editor"},
    {name: "Grok Bot", appId: "grok-bot", detail: "Chat composer"},
    {name: "Discord", appId: "discord", detail: "Needs renderer accessibility"}]

  property alias client: client
  property string query: ""
  property var serviceState: ({})
  property string serviceError: ""
  property string actionMessage: ""
  property string actionSuccess: ""
  property string actionVerb: ""
  property bool actionFailed: false
  property bool actionTimedOut: false

  readonly property bool appsBlocklist: client.settings.all_linux_apps === true
  readonly property bool sitesBlocklist: client.settings.all_web_origins === true
  readonly property bool anyAppEnabled: appsBlocklist || sitesBlocklist
    || supportedApps.some(function(app) { return appEnabled(app.appId) })
    || siteRules().some(function(rule) { return rule.allowed })
  readonly property bool ready: client.brokerReachable && !client.brokerPaused && !client.controlPlaneDegraded && anyAppEnabled
  readonly property bool paused: client.brokerReachable && client.brokerPaused
  readonly property bool controlsBusy: action.running || client.mutating
  readonly property bool canChange: client.canMutateSettings && !action.running
  readonly property string statusLabel: !client.brokerReachable
    ? (serviceState.active === "active" ? "Model starting or unavailable" : "Model offline")
    : client.controlPlaneDegraded ? "Settings need attention" : paused ? "Predictions paused"
    : !anyAppEnabled ? "Applications disabled" : "Model ready"
  readonly property var desktop: client.objectValue(client.overview, "desktop")
  readonly property var activity: client.objectValue(desktop, "activity")
  readonly property var misses: client.objectValue(desktop, "no_suggestion")
  readonly property var spelling: Array.isArray(desktop.spelling) ? desktop.spelling : []
  readonly property string modelName: {
    var advice = client.objectValue(client.objectValue(client.objectValue(client.overview, "models"), "writing"), "advice")
    var file = client.objectValue(advice, "recommended").filename
    return typeof file === "string" ? file.replace(/\.gguf$/, "").replace(/-Q[0-9A-Z_]+$/, "").replace(/-/g, " ") : ""
  }
  // The service runs but the broker does not answer yet: the model loads.
  readonly property bool loading: !client.brokerReachable
    && (serviceState.active === "active" || serviceState.active === "activating")
  // What is happening right now, for the activity line; empty when idle.
  readonly property string busyVerb: client.mutating ? "Saving"
    : action.running ? actionVerb
    : loading ? "Loading the model" : ""

  function count(key) {
    return client.integerValue(desktop, key, -1)
  }

  function wordsSaved() {
    var savings = client.objectValue(desktop, "word_savings")
    var applied = client.integerValue(savings, "applied", -1)
    var dispatched = client.integerValue(savings, "dispatched_unverified", -1)
    return applied >= 0 && dispatched >= 0 ? applied + dispatched : -1
  }

  function refresh() {
    actionMessage = ""
    client.refresh()
    probeService()
  }

  function activate() {
    client.activate()
    probeService()
  }

  function togglePause() {
    if (!client.canMutateSettings || action.running) return
    actionMessage = ""
    var document = client.cloneSettings()
    document.paused = !client.brokerPaused
    client.replaceSettings(document, document.paused ? "Paused · persists after restart" : "Resumed")
  }

  function appRule(appId) {
    var subjects = client.settings.subjects || []
    for (var i = 0; i < subjects.length; i++) {
      var entry = subjects[i]
      if (entry.identity.kind === "linux_app" && entry.identity.adapter === appAdapter(appId) && entry.identity.app_id === appId)
        return entry
    }
    return null
  }

  function allows(permissions) {
    return permissions.context_read === "allow" && permissions.suggest === "allow" && permissions.display === "allow"
  }

  // The effective state: an exact rule, else the list mode (never for manual apps).
  function appEnabled(appId) {
    var rule = appRule(appId)
    if (rule) return allows(rule.permissions)
    var app = supportedApps.find(function(item) { return item.appId === appId })
    return appsBlocklist && !(app && app.manual)
  }

  function siteRules() {
    var subjects = client.settings.subjects || []
    var rules = []
    for (var i = 0; i < subjects.length; i++) {
      var identity = subjects[i].identity
      if (identity.kind !== "browser_origin") continue
      var standard = identity.scheme === "https" ? 443 : 80
      rules.push({identity: identity, allowed: allows(subjects[i].permissions),
        origin: identity.scheme + "://" + identity.host + (identity.port === standard ? "" : ":" + identity.port)})
    }
    return rules
  }

  function appAdapter(appId) {
    return appId === "obsidian" ? "obsidian" : appId === "bash" ? "shell" : appId === "nvim" ? "neovim"
      : appId === "libreoffice-writer" ? "libreoffice" : "fcitx"
  }

  // Every write: a fresh clone of the coherent document, changed, then
  // compare-and-swap through the client.
  function change(edit, message) {
    if (!canChange) return
    actionMessage = ""
    var document = client.cloneSettings()
    edit(document)
    client.replaceSettings(document, message)
  }

  function setFlag(key, on, message) {
    change(function(document) { if (on) document[key] = true; else delete document[key] }, message)
  }

  function setEscapeQuiet(seconds) {
    change(function(document) {
      if (seconds > 0) document.escape_quiet_seconds = seconds
      else delete document.escape_quiet_seconds
    }, seconds > 0 ? "Escape keeps a field quiet for " + durationLabel(seconds) : "Escape quiet lasts until the next edit")
  }

  function setBatteryMode(mode) {
    change(function(document) {
      if (mode === "quiet") delete document.battery_mode
      else document.battery_mode = mode
    }, "On battery: " + batteryLabel(mode).toLowerCase())
  }

  readonly property var spellingOrder: ["en", "de", "fr", "es", "it", "nl", "pl"]

  function spellingLanguages() {
    var languages = client.settings.spelling_languages
    return Array.isArray(languages) ? languages : ["en"]
  }

  function setSpelling(code, on) {
    change(function(document) {
      var chosen = spellingLanguages().filter(function(item) { return item !== code })
      if (on) chosen.push(code)
      chosen = spellingOrder.filter(function(item) { return chosen.indexOf(item) >= 0 })
      if (chosen.length === 1 && chosen[0] === "en") delete document.spelling_languages
      else document.spelling_languages = chosen
    }, on ? "Spelling on" : "Spelling off")
  }

  function writeRule(identity, decision) {
    var document = client.cloneSettings()
    document.subjects = document.subjects.filter(function(item) { return client.compareIdentities(item.identity, identity) !== 0 })
    if (decision !== null)
      document.subjects.push({identity: identity, permissions: {context_read: decision, display: decision,
        suggest: decision, learn: "block", retention: {mode: "none"}}})
    document.subjects.sort(function(a, b) { return client.compareIdentities(a.identity, b.identity) })
    return document
  }

  // An exact http(s) origin, as `badi site` accepts it (ASCII hosts only here).
  function siteIdentity(text) {
    var match = /^(https?):\/\/([a-z0-9.-]+)(?::([0-9]{1,5}))?\/?$/.exec(text.trim().toLowerCase())
    if (!match) return null
    var port = match[3] ? Number(match[3]) : (match[1] === "https" ? 443 : 80)
    if (port < 1 || port > 65535 || match[2].length > 253) return null
    return {kind: "browser_origin", adapter: "chromium", scheme: match[1], host: match[2], port: port}
  }

  function setSite(text, decision) {
    if (!canChange) return false
    var identity = siteIdentity(text)
    if (!identity) {
      actionMessage = "Enter an exact site such as https://mail.example.com"
      actionFailed = true
      return false
    }
    actionMessage = ""
    client.replaceSettings(writeRule(identity, decision), decision === null ? "Site rule removed"
      : decision === "allow" ? "Site allowed" : "Site blocked")
    return true
  }

  function toggleApp(appId) {
    if (!canChange) return
    actionMessage = ""
    var enabled = !appEnabled(appId)
    var identity = {kind: "linux_app", adapter: appAdapter(appId), app_id: appId}
    client.replaceSettings(writeRule(identity, enabled ? "allow" : "block"), enabled ? "App allowed" : "App blocked")
  }

  function runAction(arguments, verb, success) {
    if (controlsBusy) return
    actionVerb = verb
    actionMessage = ""
    actionSuccess = success
    actionFailed = false
    action.exec([root.helper].concat(arguments))
  }

  function probeService() {
    if (!serviceProbe.running) serviceProbe.exec([root.helper, "service", "status"])
  }

  function durationLabel(seconds) {
    return seconds >= 3600 ? seconds / 3600 + " h" : seconds >= 60 ? seconds / 60 + " min" : seconds + " s"
  }

  function batteryLabel(mode) {
    return mode === "normal" ? "As on mains" : mode === "explicit" ? "Only when asked" : "After a pause"
  }

  // ---- Rows ---------------------------------------------------------------
  // kind: toggle (activate flips), choice (h/l or a chip picks), action
  // (activate runs), site (activate flips allow/block, x removes) and field
  // (activate focuses the text input). Every row names its section.

  function rows() {
    var list = []
    var reachable = client.brokerReachable

    list.push({key: "mid_line", section: "Writing", kind: "toggle", label: "Mid-sentence suggestions",
      detail: "Also suggest when a word follows the caret", checked: client.settings.mid_line === true,
      enabled: canChange, activate: function() { root.setFlag("mid_line", client.settings.mid_line !== true,
        client.settings.mid_line === true ? "Mid-sentence suggestions off" : "Mid-sentence suggestions on") }})
    var quiet = client.integerValue(client.settings, "escape_quiet_seconds", 0)
    list.push({key: "escape_quiet", section: "Writing", kind: "choice", label: "Escape keeps a field quiet",
      detail: "Ctrl+Shift+Space always asks again", value: String(quiet), enabled: canChange,
      options: [{value: "0", label: "Until edit"}, {value: "30", label: "30 s"}, {value: "300", label: "5 min"}, {value: "3600", label: "1 h"}],
      choose: function(value) { if (Number(value) !== quiet) root.setEscapeQuiet(Number(value)) }})
    var battery = client.settings.battery_mode || "quiet"
    list.push({key: "battery", section: "Writing", kind: "choice", label: "On battery",
      detail: batteryLabel(battery) + " · explicit requests never change", value: battery, enabled: canChange,
      options: [{value: "quiet", label: "Pause first"}, {value: "normal", label: "As on mains"}, {value: "explicit", label: "Ask only"}],
      choose: function(value) { if (value !== battery) root.setBatteryMode(value) }})

    var chosen = spellingLanguages()
    var report = {}
    for (var r = 0; r < spelling.length; r++) report[spelling[r].language] = spelling[r]
    var names = {en: "English", de: "German", fr: "French", es: "Spanish", it: "Italian", nl: "Dutch", pl: "Polish"}
    spellingOrder.forEach(function(code) {
      var entry = report[code] || {}
      var on = chosen.indexOf(code) >= 0
      var installed = typeof entry.dictionary === "string"
      list.push({key: "spelling_" + code, section: "Spelling", kind: "toggle", label: names[code],
        detail: code === "en" ? "Built in" : installed ? "Dictionary " + entry.dictionary
          : "Needs sudo pacman -S " + (entry.package || "its hunspell dictionary"),
        warn: on && !installed && code !== "en", checked: on, enabled: canChange,
        activate: function() { root.setSpelling(code, !on) }})
    })

    list.push({key: "apps_mode", section: "Apps", kind: "choice", label: "Apps",
      detail: appsBlocklist ? "Every supported app unless blocked" : "Only the apps you allow",
      value: appsBlocklist ? "all" : "allowed", enabled: canChange,
      options: [{value: "allowed", label: "Allowed only"}, {value: "all", label: "All but blocked"}],
      choose: function(value) { if ((value === "all") !== appsBlocklist) root.setFlag("all_linux_apps", value === "all",
        value === "all" ? "Apps: all but blocked" : "Apps: allowed only") }})
    supportedApps.forEach(function(app) {
      list.push({key: "app_" + app.appId, section: "Apps", kind: "toggle", label: app.name,
        detail: app.detail + (appRule(app.appId) ? "" : " · follows the mode"),
        checked: reachable && appEnabled(app.appId), enabled: canChange,
        activate: function() { root.toggleApp(app.appId) }})
    })

    list.push({key: "sites_mode", section: "Websites", kind: "choice", label: "Websites",
      detail: "Chromium, Brave and Zen · passwords always refused",
      value: sitesBlocklist ? "all" : "allowed", enabled: canChange,
      options: [{value: "allowed", label: "Allowed only"}, {value: "all", label: "All but blocked"}],
      choose: function(value) { if ((value === "all") !== sitesBlocklist) root.setFlag("all_web_origins", value === "all",
        value === "all" ? "Websites: all but blocked" : "Websites: allowed only") }})
    siteRules().forEach(function(rule) {
      list.push({key: "site_" + rule.origin, section: "Websites", kind: "site", label: rule.origin,
        detail: rule.allowed ? "Allowed · enter blocks · x removes" : "Blocked · enter allows · x removes",
        checked: rule.allowed, enabled: canChange,
        activate: function() { root.setSite(rule.origin, rule.allowed ? "block" : "allow") },
        remove: function() { root.setSite(rule.origin, null) }})
    })
    list.push({key: "site_add", section: "Websites", kind: "field", label: "Add a site",
      detail: "Enter allows · Shift+Enter blocks", enabled: canChange})

    var autostart = serviceState.autostart
    list.push({key: "autostart", section: "System", kind: "toggle", label: "Start at login",
      detail: typeof autostart === "boolean" ? "The model loads with your session" : "Service status unavailable",
      checked: autostart === true, enabled: !controlsBusy && typeof autostart === "boolean",
      activate: function() { root.runAction(["autostart", autostart ? "off" : "on"], "Updating startup", "Startup preference saved") }})
    list.push({key: "debug", section: "System", kind: "toggle", label: "Debug trace",
      detail: activity.enabled ? "Recording · badi debug watch follows it" : "15 minutes of events, no typed text",
      checked: activity.enabled === true, enabled: !controlsBusy,
      activate: function() { root.runAction(["debug", activity.enabled ? "off" : "on"], "Updating the trace", activity.enabled ? "Trace off" : "Trace on · badi debug watch") }})
    var running = serviceState.active === "active"
    list.push({key: "restart", section: "System", kind: "action", label: "Restart the model",
      detail: "Reloads the local model; settings stay", enabled: !controlsBusy,
      activate: function() { root.runAction(["service", "restart"], "Restarting the model", "Model restarted") }})
    list.push({key: "service", section: "System", kind: "action", label: running ? "Stop the model" : "Start the model",
      detail: running ? "Frees its memory until you start it" : "Loads the local model now", enabled: !controlsBusy,
      activate: function() { root.runAction(["service", running ? "stop" : "start"], running ? "Stopping the model" : "Starting the model",
        running ? "Model stopped" : "Model started") }})

    var needle = query.trim().toLowerCase()
    if (!needle) return list
    return list.filter(function(row) {
      return (row.label + " " + row.detail + " " + row.section).toLowerCase().indexOf(needle) >= 0
    })
  }

  BadiClient {
    id: client
    cliPrefix: [root.helper, "ctl"]
  }

  Timer { id: actionTimeout; interval: 20000; onTriggered: if (action.running) { root.actionTimedOut = true; action.signal(15); actionKill.restart() } }
  Timer { id: actionKill; interval: 1000; onTriggered: if (action.running) action.signal(9) }
  Process {
    id: action
    stdout: StdioCollector {}
    stderr: StdioCollector { id: actionError }
    onStarted: { root.actionTimedOut = false; actionTimeout.restart() }
    onExited: (exitCode) => {
      actionTimeout.stop(); actionKill.stop()
      root.actionFailed = exitCode !== 0 || root.actionTimedOut
      root.actionMessage = root.actionTimedOut ? "Timed out · run badi doctor"
        : exitCode === 0 ? root.actionSuccess : actionError.text.trim() || "Could not complete the action"
      client.refresh(true); root.probeService()
    }
  }
  Timer { id: probeTimeout; interval: 6000; onTriggered: if (serviceProbe.running) { serviceProbe.signal(15); probeKill.restart() } }
  Timer { id: probeKill; interval: 1000; onTriggered: if (serviceProbe.running) serviceProbe.signal(9) }
  Process {
    id: serviceProbe
    stdout: StdioCollector { id: serviceOutput }
    stderr: StdioCollector { id: serviceStderr }
    onStarted: probeTimeout.restart()
    onExited: (exitCode) => {
      probeTimeout.stop(); probeKill.stop()
      try {
        var state = JSON.parse(serviceOutput.text)
        if (exitCode !== 0 || typeof state.autostart !== "boolean" || typeof state.active !== "string") throw new Error("invalid service state")
        root.serviceState = state
        root.serviceError = state.startup_problem && typeof state.startup_problem.message === "string"
          ? state.startup_problem.message : ""
      } catch (error) {
        root.serviceState = ({})
        root.serviceError = serviceStderr.text.trim() || "Service status unavailable · run badi doctor"
      }
    }
  }

  Component.onCompleted: { client.activate(); probeService() }
  Component.onDestruction: {
    client.dispose()
    if (action.running) action.signal(9)
    if (serviceProbe.running) serviceProbe.signal(9)
  }
}
