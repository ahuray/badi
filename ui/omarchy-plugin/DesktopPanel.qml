import QtQuick
import Quickshell
import Quickshell.Io
import "KeyModel.js" as KeyModel
import "StatsModel.js" as StatsModel

// The panel's state and actions, without any visuals: the broker client,
// the desktop helper's processes, every settings change (compare-and-swap
// through BadiClient) and the typed rows SettingsView renders. Rows are
// plain descriptors, so search and keyboard navigation work on one list.
Item {
  id: root

  // Found on the shell's PATH: /usr/bin from a package, ~/.local/bin from
  // the user-local installer (the two are not meant to be mixed).
  readonly property string helper: "badi-desktop"
  // Supported apps; `manual` apps need an exact rule even in blocklist mode.
  // `context` says what context finishes words from there (ADR 0010): the
  // window and learned words (web content), learned words only (native
  // fields), or the typed line only (terminals, which never learn).
  readonly property var supportedApps: [
    {name: "Omawrite", appId: "omawrite", context: "learned", detail: "Markdown editor"},
    {name: "Ghostty", appId: "com.mitchellh.ghostty", context: "line", detail: "Terminal · follows the line you type", manual: true},
    {name: "foot", appId: "foot", context: "line", detail: "Terminal · follows the line you type", manual: true},
    {name: "Xournal++", appId: "com.github.xournalpp.xournalpp", context: "learned", detail: "Text tool · Tab requests, Tab again accepts a word", manual: true},
    {name: "Obsidian", appId: "obsidian", context: "learned", detail: "Badi plugin · Tab a word, Ctrl+Right all"},
    {name: "Neovim", appId: "nvim", context: "learned", detail: "Badi plugin · prose buffers"},
    {name: "Bash", appId: "bash", context: "line", detail: "Ctrl-X Tab requests and accepts"},
    {name: "LibreOffice Writer", appId: "libreoffice", context: "learned", detail: "Grey preview in paragraphs"},
    {name: "Writer spelling", appId: "libreoffice-writer", detail: "Corrections after a word · Alt+F7"},
    {name: "Telegram", appId: "telegram", context: "learned", detail: "Message fields"},
    {name: "Codex", appId: "chatgpt", context: "window", detail: "Composer"},
    {name: "VS Code", appId: "code", context: "window", detail: "Needs \"editor.editContext\": false"},
    {name: "Cursor", appId: "cursor", context: "window", detail: "Editor"},
    {name: "Grok Bot", appId: "grok-bot", context: "window", detail: "Chat composer"},
    {name: "Claude", appId: "com.anthropic.claude", context: "window", detail: "Desktop app composer"},
    {name: "Discord", appId: "discord", context: "window", detail: "Needs renderer accessibility"}]

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
  // Usage statistics (badi stats): counts per local day and per app or site,
  // kept 90 days. Absent from a broker that has none.
  readonly property var stats: client.objectValue(desktop, "stats")
  readonly property bool statsReported: typeof stats.available === "boolean"
  readonly property bool statsAvailable: stats.available === true
  readonly property var statsSummary: client.objectValue(stats, "summary")
  readonly property var modelAdvice: client.objectValue(client.objectValue(client.objectValue(client.overview, "models"), "writing"), "advice")
  readonly property var modelCatalog: Array.isArray(modelAdvice.catalog) ? modelAdvice.catalog : []
  // Terms that come with a model's licence; a model with terms is chosen,
  // then downloaded only from the row that names them.
  readonly property var modelTerms: ({
    "Gemma": {name: "the Gemma Terms of Use", url: "ai.google.dev/gemma/terms"},
    "Llama-3.2-Community": {name: "the Llama 3.2 Community License", url: "llama.com/llama3_2/license"}
  })
  property string pendingModel: ""
  // The model the running broker loaded, else the device recommendation.
  readonly property var activeModel: {
    for (var i = 0; i < modelCatalog.length; i++)
      if (modelCatalog[i].filename === desktop.model_file) return modelCatalog[i]
    return client.objectValue(modelAdvice, "recommended")
  }
  readonly property string modelName: typeof activeModel.name === "string" ? activeModel.name : ""
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

  // The history's numbers (StatsModel.js): a window's counts, the week's
  // words per day and the apps and sites that saved the most this week.
  function statsTotals(window) {
    return StatsModel.totals(statsSummary, window)
  }

  function statsWeek() {
    return StatsModel.week(statsSummary)
  }

  function statsLeaders(count) {
    var names = {}
    supportedApps.forEach(function(app) { names[app.appId] = app.name })
    return StatsModel.leaders(statsSummary, names, count)
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

  // Context is off for this app (badi context app ID off).
  function contextOff(appId) {
    var off = client.settings.context_words_off || []
    for (var i = 0; i < off.length; i++) {
      if (off[i].kind === "linux_app" && off[i].adapter === appAdapter(appId) && off[i].app_id === appId) return true
    }
    return false
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

  readonly property var writingOrder: ["en", "de", "fa", "fr", "es", "pt"]

  function writingLanguages() {
    var languages = client.settings.writing_languages
    return Array.isArray(languages) ? languages : ["en", "de", "fa"]
  }

  // One language's choice: off, or on with healing auto, forced on or off.
  function setWriting(code, value) {
    change(function(document) {
      var chosen = writingLanguages().filter(function(item) { return item !== code })
      if (value !== "off") chosen.push(code)
      chosen = writingOrder.filter(function(item) { return chosen.indexOf(item) >= 0 })
      if (chosen.join() === "en,de,fa") delete document.writing_languages
      else document.writing_languages = chosen
      var healing = client.objectValue(document, "trailing_space_healing")
      var choices = {}
      Object.keys(healing).forEach(function(key) { if (key !== code) choices[key] = healing[key] })
      if (value === "heal") choices[code] = true
      if (value === "plain") choices[code] = false
      if (Object.keys(choices).length === 0) delete document.trailing_space_healing
      else document.trailing_space_healing = choices
    }, value === "off" ? "Suggestions off" : "Suggestions on")
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

  // `timeout` in ms; a model download needs minutes, the rest seconds.
  function runAction(arguments, verb, success, timeout) {
    if (controlsBusy) return
    actionTimeout.interval = timeout || 20000
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

    // The writing model: every model that fits this device, the best
    // measured one marked; picking one downloads it (pinned) and restarts.
    var fitting = modelCatalog.filter(function(model) { return model.fits === true })
    var recommendedId = client.objectValue(modelAdvice, "recommended").id
    if (fitting.length > 0) {
      var licences = {"Gemma": "Gemma terms", "Llama-3.2-Community": "Llama 3.2 licence", "Apache-2.0": "Apache-2.0"}
      var pending = fitting.filter(function(model) { return model.id === root.pendingModel })[0]
      list.push({key: "model", section: "Model", kind: "choice", label: "Writing model",
        detail: (activeModel.id === recommendedId ? "Recommended for this device" : "Recommended: " + (client.objectValue(modelAdvice, "recommended").name || "none"))
          + " · " + (licences[activeModel.license] || activeModel.license || ""),
        value: pending ? pending.id : activeModel.id || "", enabled: !controlsBusy,
        options: fitting.map(function(model) {
          return {value: model.id, label: model.name + (model.id === recommendedId ? " ★" : "")}
        }),
        choose: function(value) {
          var chosen = fitting.filter(function(model) { return model.id === value })[0]
          if (value === activeModel.id || !chosen) { root.pendingModel = ""; return }
          if (root.modelTerms[chosen.license]) { root.pendingModel = value; return }
          root.pendingModel = ""
          root.runAction(["model", "use", value], "Switching the model", "Model switched", 900000)
        }})
      if (pending && root.modelTerms[pending.license]) {
        var terms = root.modelTerms[pending.license]
        list.push({key: "model-terms", section: "Model", kind: "action",
          label: "Accept " + terms.name + " and download " + pending.name,
          detail: terms.url + " · " + (pending.download_bytes / 1e9).toFixed(1) + " GB, pinned and hash-checked",
          enabled: !controlsBusy,
          activate: function() {
            root.pendingModel = ""
            root.runAction(["model", "use", pending.id, "--accept-license"], "Switching the model", "Model switched", 900000)
          }})
      }
    }
    // Opt-in context (ADR 0010): asked once while unchosen, then a switch,
    // one row per allowed app and a way to forget the learned words.
    var context = client.settings.context_words
    if (reachable && typeof context !== "boolean") {
      list.push({key: "context_prompt", section: "Context", kind: "choice", label: "Finish words from what you see?",
        detail: "Words from the window or your own writing · on this device",
        value: "", enabled: !controlsBusy,
        options: [{value: "on", label: "Turn on"}, {value: "off", label: "Not now"}],
        choose: function(value) { root.runAction(["context", value], value === "on" ? "Turning context on" : "Saving your choice",
          value === "on" ? "Context on · badi context status" : "Context stays off") }})
    } else {
      list.push({key: "context", section: "Context", kind: "toggle", label: "Words from context",
        detail: context === true ? "Window words and words you wrote before · sealed in your keyring"
          : "Off · finish words from the window and your own writing", checked: context === true,
        enabled: !controlsBusy, activate: function() {
          root.runAction(["context", context === true ? "off" : "on"], "Updating context",
            context === true ? "Context off" : "Context on") }})
    }
    if (context === true) {
      var sources = {window: "Window words and words learned here", learned: "Words learned here · the window is not read",
        line: "The line you type only · never learns here"}
      supportedApps.filter(function(app) { return app.context && appEnabled(app.appId) }).forEach(function(app) {
        var on = !contextOff(app.appId)
        list.push({key: "context_app_" + app.appId, section: "Context", kind: "toggle", label: app.name,
          detail: on ? sources[app.context] : "Off here · its learned words are deleted",
          checked: on, enabled: !controlsBusy,
          activate: function() { root.runAction(["context", "app", app.appId, on ? "off" : "on"], "Updating context",
            app.name + (on ? ": context off" : ": context on")) }})
      })
      list.push({key: "context_forget", section: "Context", kind: "action", label: "Forget learned words",
        detail: "Deletes every learned word; the keyring key stays", enabled: !controlsBusy,
        activate: function() { root.runAction(["context", "forget"], "Forgetting learned words", "Learned words deleted") }})
    }
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

    var writing = writingLanguages()
    var healingChoices = client.objectValue(client.settings, "trailing_space_healing")
    var modelHealed = Array.isArray(activeModel.healed_languages) ? activeModel.healed_languages : ["en", "fa"]
    var writingNames = {en: "English", de: "German", fa: "Persian", fr: "French", es: "Spanish", pt: "Portuguese"}
    writingOrder.forEach(function(code) {
      var on = writing.indexOf(code) >= 0
      var choice = healingChoices[code]
      var value = !on ? "off" : choice === true ? "heal" : choice === false ? "plain" : "auto"
      var healed = choice === true || (choice !== false && modelHealed.indexOf(code) >= 0)
      list.push({key: "writing_" + code, section: "Suggestions", kind: "choice", label: writingNames[code],
        detail: (["fr", "es", "pt"].indexOf(code) >= 0 ? "Experimental · " : "")
          + (!on ? "No suggestions" : healed ? "Heals the space after a word" : "Nothing after a typed space")
          + (value === "auto" && modelName ? " · as reviewed for " + modelName : ""),
        value: value, enabled: canChange,
        options: [{value: "off", label: "Off"}, {value: "auto", label: "Auto"},
          {value: "heal", label: "Heal"}, {value: "plain", label: "Keep space"}],
        choose: function(next) { if (next !== value) root.setWriting(code, next) }})
    })

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

    // Usage statistics: counts only, per day and app or site (badi stats).
    if (reachable && statsReported) {
      list.push({key: "stats_forget", section: "Statistics", kind: "action", label: "Forget usage statistics",
        detail: statsAvailable ? "Counts per day and app, kept 90 days · badi stats"
          : "⚠ Unreadable and kept as it is · forgetting starts the count again",
        enabled: !controlsBusy,
        activate: function() { root.runAction(["stats", "forget"], "Forgetting usage statistics", "Usage statistics deleted") }})
    }

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

    return KeyModel.filter(list, query)
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
