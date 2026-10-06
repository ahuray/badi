import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Ui
import qs.Commons

Item {
  id: root
  readonly property string helper: Quickshell.env("HOME") + "/.local/bin/badi-desktop"
  // Supported apps; `manual` apps (Tab requests) need an exact rule even in blocklist mode.
  readonly property var supportedApps: [
    {name: "Omawrite", appId: "omawrite", detail: "Markdown editor · automatic"},
    {name: "Xournal++", appId: "com.github.xournalpp.xournalpp", detail: "Text tool · Tab requests, Tab again accepts", manual: true},
    {name: "Obsidian", appId: "obsidian", detail: "Badi plugin · Tab accepts a word · Ctrl/Command+Right accepts all"},
    {name: "Bash", appId: "bash", detail: "Ctrl-X Tab requests / accepts"},
    {name: "Telegram", appId: "telegram", detail: "Message fields · Fcitx panel"},
    {name: "LibreOffice Writer", appId: "libreoffice", detail: "Document paragraphs · Fcitx panel"},
    {name: "Codex", appId: "chatgpt", detail: "Composer"},
    {name: "VS Code", appId: "code", detail: "Editor · needs \"editor.editContext\": false"},
    {name: "Cursor", appId: "cursor", detail: "Editor"},
    {name: "Grok Bot", appId: "grok-bot", detail: "Chat composer"},
    {name: "Discord", appId: "discord", detail: "Needs renderer accessibility, which Discord resets"}]
  readonly property bool appsBlocklist: client.settings.all_linux_apps === true
  readonly property bool sitesBlocklist: client.settings.all_web_origins === true
  readonly property bool anyAppEnabled: appsBlocklist || sitesBlocklist
    || supportedApps.some(function(app) { return appEnabled(app.appId) })
    || siteRules().some(function(rule) { return rule.allowed })
  readonly property bool ready: client.brokerReachable && !client.brokerPaused && !client.controlPlaneDegraded && anyAppEnabled
  readonly property bool paused: client.brokerReachable && client.brokerPaused
  readonly property string statusLabel: !client.brokerReachable
    ? (serviceState.active === "active" ? "Model starting or unavailable" : "Model offline")
    : client.controlPlaneDegraded ? "Settings need attention" : paused ? "Predictions paused"
    : !anyAppEnabled ? "Applications disabled" : "Model ready"
  readonly property var activity: client.overview.desktop ? client.overview.desktop.activity || ({}) : ({})
  property int page: 0
  property var serviceState: ({})
  property string serviceError: ""
  property string actionMessage: ""
  property string actionSuccess: ""
  property bool actionFailed: false
  property bool actionTimedOut: false
  readonly property bool controlsBusy: action.running || client.mutating
  readonly property string modelName: client.broker.provider === "local_model"
    ? "Local writing model · CPU" : "Provider: " + (client.broker.provider || "offline")

  IpcHandler {
    target: "badi-writing"
    function toggle(): void { root.toggle() }
    function open(): void { root.show() }
    function state(): string {
      return JSON.stringify({visible: window.visible, minimized: window.minimized,
        status: root.statusLabel, page: root.page, paused: root.paused, service: root.serviceState})
    }
  }

  function toggle() {
    if (window.visible && !window.minimized) window.visible = false
    else show()
  }

  function show() {
    window.minimized = false
    window.visible = true
    client.refresh(); probeService(); closeButton.forceActiveFocus()
  }

  function togglePause() {
    if (!client.canMutateSettings || action.running) return
    actionMessage = ""
    var document = client.cloneSettings()
    document.paused = !client.brokerPaused
    client.replaceSettings(document, document.paused ? "Predictions paused. This persists after restart." : "Predictions resumed.")
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

  function setMode(key, blocklist) {
    if (!client.canMutateSettings || action.running) return
    actionMessage = ""
    var document = client.cloneSettings()
    if (blocklist) document[key] = true
    else delete document[key]
    var apps = key === "all_linux_apps"
    client.replaceSettings(document, blocklist
      ? (apps ? "Apps: every supported app except blocked ones." : "Websites: every site except blocked ones. Sensitive fields stay denied.")
      : (apps ? "Apps: only allowed apps." : "Websites: only allowed sites."))
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
    if (!client.canMutateSettings || action.running) return
    var identity = siteIdentity(text)
    if (!identity) {
      actionMessage = "Enter an exact site such as https://mail.example.com"
      actionFailed = true
      return
    }
    actionMessage = ""
    client.replaceSettings(writeRule(identity, decision), decision === null ? "Site rule removed; it follows the website mode."
      : decision === "allow" ? "Site allowed." : "Site blocked. Context reading and predictions are disabled there.")
  }

  function appAdapter(appId) {
    return appId === "obsidian" ? "obsidian" : appId === "bash" ? "shell" : "fcitx"
  }

  function toggleApp(appId) {
    if (!client.canMutateSettings || action.running) return
    actionMessage = ""
    var enabled = !appEnabled(appId)
    var identity = {kind: "linux_app", adapter: appAdapter(appId), app_id: appId}
    client.replaceSettings(writeRule(identity, enabled ? "allow" : "block"),
      enabled ? "App enabled. Predictions are available in supported text fields." : "App blocked. Context reading and predictions are disabled.")
  }

  function runAction(arguments, progress, success) {
    if (controlsBusy) return
    actionMessage = progress
    actionSuccess = success
    actionFailed = false
    action.exec([root.helper].concat(arguments))
  }

  function probeService() {
    if (!serviceProbe.running) serviceProbe.exec([root.helper, "service", "status"])
  }

  BadiClient {
    id: client
    cliPrefix: [root.helper, "ctl"]
  }

  Timer {
    interval: window.visible ? 3000 : 10000
    running: true; repeat: true
    onTriggered: { client.refresh(true); root.probeService() }
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
      root.actionMessage = root.actionTimedOut ? "Action timed out. Run badi doctor to check the result."
        : exitCode === 0 ? root.actionSuccess : "Could not complete action: " + actionError.text.trim()
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
          ? state.startup_problem.message + " Run badi doctor for recovery steps." : ""
      } catch (error) {
        root.serviceState = ({})
        root.serviceError = serviceStderr.text.trim() || "Service status unavailable. Run badi doctor."
      }
    }
  }

  component ActionButton: Button {
    opacity: enabled ? 1 : 0.5
    Accessible.role: Accessible.Button
    Accessible.name: text
    Accessible.onPressAction: clicked()
  }

  component BodyText: Text {
    Layout.fillWidth: true
    textFormat: Text.PlainText
    wrapMode: Text.WordWrap
    color: Color.popups.text
    font.family: Style.font.family
    font.pixelSize: Style.font.body
  }
  component Caption: BodyText { font.pixelSize: Style.font.caption; opacity: 0.8 }
  component SectionTitle: BodyText { font.bold: true; font.pixelSize: Style.font.heading }

  FloatingWindow {
    id: window
    title: "Badi writing settings"
    visible: false
    color: Color.popups.background
    implicitWidth: Style.space(560)
    implicitHeight: Style.space(680)
    minimumSize: Qt.size(Style.space(420), Style.space(450))

    FocusScope {
      anchors.fill: parent
      focus: true
      Keys.onEscapePressed: window.visible = false
      ColumnLayout {
        anchors.fill: parent
        anchors.margins: Style.space(24)
        spacing: Style.space(16)
        RowLayout {
          Layout.fillWidth: true
          Text { text: "Badi"; color: Color.popups.text; font.family: Style.font.family; font.pixelSize: Style.font.title; font.bold: true }
          Item { Layout.fillWidth: true }
          ActionButton { id: closeButton; text: "Close"; focusable: true; onClicked: window.visible = false; Accessible.name: "Close Badi settings" }
        }
        RowLayout {
          Layout.fillWidth: true
          Repeater {
            model: ["Writing", "Applications", "System"]
            ActionButton {
              required property int index
              required property string modelData
              Layout.fillWidth: true
              text: modelData; selected: root.page === index; bordered: true; focusable: true
              onClicked: root.page = index
              Accessible.name: modelData + " settings"
            }
          }
        }
        Controls.ScrollView {
          id: scroll
          Layout.fillWidth: true; Layout.fillHeight: true
          clip: true
          Controls.ScrollBar.horizontal.policy: Controls.ScrollBar.AlwaysOff
          ColumnLayout {
            width: scroll.availableWidth
            spacing: Style.space(16)
            ColumnLayout {
              visible: root.page === 0
              Layout.fillWidth: true
              spacing: Style.space(16)
              SectionTitle { text: root.statusLabel; color: root.ready ? Color.accent : Color.popups.text }
              Caption { text: client.brokerReachable ? root.modelName : "Start the model to use local predictions." }
              ActionButton {
                Layout.fillWidth: true
                text: client.brokerReachable ? (root.paused ? "Resume predictions" : "Pause predictions") : "Start model"
                bordered: true; focusable: true
                enabled: !root.controlsBusy && (client.brokerReachable ? client.canMutateSettings : true)
                onClicked: client.brokerReachable ? root.togglePause() : root.runAction(["service", "start"], "Starting model…", "Model process started. Waiting for it to load…")
              }
              Caption { text: "Pause keeps the model loaded. Your choice persists after restart." }
              PanelSeparator { Layout.fillWidth: true; foreground: Color.popups.text }
              SectionTitle { text: "Try a continuation" }
              BodyText { text: "Suggestions appear on their own in supported apps: Tab inserts a visible suggestion, Ctrl+Right inserts its next word, typing its next letters keeps the rest, Escape dismisses, Ctrl+Shift+Space asks explicitly. In a Xournal++ text cell, press Tab to request words and Tab again to insert them. Obsidian: Tab takes one word, Ctrl/Command+Right takes all." }
              RowLayout {
                ActionButton { text: "Open Omawrite"; bordered: true; focusable: true; enabled: !root.controlsBusy; onClicked: root.runAction(["launch", "omawrite"], "Opening Omawrite…", "Omawrite opened. Type a phrase; press Tab to insert the suggestion.") }
                ActionButton { text: "Open Xournal++"; bordered: true; focusable: true; enabled: !root.controlsBusy; onClicked: root.runAction(["launch", "xournalpp"], "Opening Xournal++…", "Xournal++ opened. Select the Text tool and click the page.") }
              }
              Caption { text: "Example: Please find attached the\nEscape dismisses. Suggestions last five seconds and clear when focus changes or the text stops matching them." }
              Caption {
                visible: client.brokerReachable
                text: client.overview.desktop ? "Requests  " + client.overview.desktop.requests + "     Suggestions  " + client.overview.desktop.suggestions + "     Errors  " + client.overview.desktop.errors : ""
              }
              Caption { text: "Local inference · No cloud · No clipboard or screen reading\nNative app text is not retained or used for learning." }
            }
            ColumnLayout {
              visible: root.page === 1
              Layout.fillWidth: true
              spacing: Style.space(16)
              SectionTitle { text: "Applications" }
              RowLayout {
                Layout.fillWidth: true
                ActionButton {
                  Layout.fillWidth: true
                  text: "Only allowed apps"; selected: !root.appsBlocklist; bordered: true; focusable: true
                  enabled: client.canMutateSettings && !action.running
                  onClicked: if (root.appsBlocklist) root.setMode("all_linux_apps", false)
                  Accessible.name: "Applications allowlist: only allowed apps"
                }
                ActionButton {
                  Layout.fillWidth: true
                  text: "All except blocked"; selected: root.appsBlocklist; bordered: true; focusable: true
                  enabled: client.canMutateSettings && !action.running
                  onClicked: if (!root.appsBlocklist) root.setMode("all_linux_apps", true)
                  Accessible.name: "Applications blocklist: every supported app except blocked ones"
                }
              }
              Caption { text: root.appsBlocklist
                ? "Every supported app works unless you block it. Badi then opens only fields it can verify, so Xournal++'s Tab requests still need Allow."
                : "Badi works only in the apps you allow." }
              Repeater {
                model: root.supportedApps
                ColumnLayout {
                  required property var modelData
                  Layout.fillWidth: true
                  RowLayout {
                    Layout.fillWidth: true
                    BodyText { text: modelData.name; font.bold: true }
                    ActionButton {
                      text: !client.brokerReachable ? "Unavailable" : root.appEnabled(modelData.appId) ? "Allowed · Block" : "Blocked · Allow"
                      bordered: true; focusable: true; enabled: client.canMutateSettings && !action.running
                      onClicked: root.toggleApp(modelData.appId)
                      Accessible.name: modelData.name + " predictions: " + (root.appEnabled(modelData.appId) ? "allowed; activate to block" : "blocked; activate to allow")
                    }
                  }
                  Caption { text: modelData.detail + (root.appRule(modelData.appId) ? "" : " · follows the list mode") }
                }
              }
              PanelSeparator { Layout.fillWidth: true; foreground: Color.popups.text }
              SectionTitle { text: "Websites" }
              RowLayout {
                Layout.fillWidth: true
                ActionButton {
                  Layout.fillWidth: true
                  text: "Only allowed sites"; selected: !root.sitesBlocklist; bordered: true; focusable: true
                  enabled: client.canMutateSettings && !action.running
                  onClicked: if (root.sitesBlocklist) root.setMode("all_web_origins", false)
                  Accessible.name: "Websites allowlist: only allowed sites"
                }
                ActionButton {
                  Layout.fillWidth: true
                  text: "All except blocked"; selected: root.sitesBlocklist; bordered: true; focusable: true
                  enabled: client.canMutateSettings && !action.running
                  onClicked: if (!root.sitesBlocklist) root.setMode("all_web_origins", true)
                  Accessible.name: "Websites blocklist: every site except blocked ones"
                }
              }
              Caption { text: "Applies to Chromium, Brave (and its web apps) and Zen, private windows included. Password and sensitive fields are always refused." }
              Repeater {
                model: root.siteRules()
                RowLayout {
                  required property var modelData
                  Layout.fillWidth: true
                  BodyText { text: modelData.origin + (modelData.allowed ? " · allowed" : " · blocked") }
                  ActionButton {
                    text: "Remove"; bordered: true; focusable: true; enabled: client.canMutateSettings && !action.running
                    onClicked: root.setSite(modelData.origin, null)
                    Accessible.name: "Remove the rule for " + modelData.origin
                  }
                }
              }
              RowLayout {
                Layout.fillWidth: true
                TextField {
                  id: siteField
                  Layout.fillWidth: true
                  placeholderText: "https://mail.example.com"
                  Accessible.name: "Website to allow or block"
                }
                ActionButton {
                  text: "Allow"; bordered: true; focusable: true; enabled: client.canMutateSettings && !action.running && siteField.text.length > 0
                  onClicked: { root.setSite(siteField.text, "allow"); siteField.text = "" }
                }
                ActionButton {
                  text: "Block"; bordered: true; focusable: true; enabled: client.canMutateSettings && !action.running && siteField.text.length > 0
                  onClicked: { root.setSite(siteField.text, "block"); siteField.text = "" }
                }
              }
              Caption { text: "Terminal: badi app list, badi site list, badi apps|sites allowlist|blocklist, badi app|site NAME on|off|reset." }
            }
            ColumnLayout {
              visible: root.page === 2
              Layout.fillWidth: true
              spacing: Style.space(16)
              SectionTitle { text: "Desktop service" }
              RowLayout {
                Layout.fillWidth: true
                BodyText { text: "Start at login" }
                ActionButton {
                  text: typeof root.serviceState.autostart !== "boolean" ? "Unavailable" : root.serviceState.autostart ? "On · Disable" : "Off · Enable"
                  bordered: true; focusable: true
                  enabled: !root.controlsBusy && typeof root.serviceState.autostart === "boolean"
                  onClicked: root.runAction(["autostart", root.serviceState.autostart ? "off" : "on"], "Updating startup…", "Startup preference saved. The current model process is unchanged.")
                  Accessible.name: "Start Badi at login: " + (root.serviceState.autostart ? "on" : "off")
                }
              }
              Caption { text: "Stopping the model frees its memory. Pause predictions when you want a quick break." }
              RowLayout {
                Layout.fillWidth: true
                BodyText { text: "Activity debug" }
                ActionButton {
                  text: root.activity.enabled ? "On · Disable" : "Off · Enable"
                  bordered: true; focusable: true; enabled: !root.controlsBusy
                  onClicked: root.runAction(["debug", root.activity.enabled ? "off" : "on"], "Updating diagnostics…", "Diagnostics updated.")
                }
              }
              Caption { text: root.activity.enabled
                ? "Last activity: " + (root.activity.reason || "waiting") + ". Full counters: badi debug watch"
                : "Trace input, requests and acceptance for 15 minutes. Typed text is never recorded." }
              RowLayout {
                ActionButton { text: "Restart model"; bordered: true; focusable: true; enabled: !root.controlsBusy; onClicked: root.runAction(["service", "restart"], "Restarting model…", "Model restarted. Waiting for it to load…") }
                ActionButton { text: root.serviceState.active === "active" ? "Stop model" : "Start model"; bordered: true; focusable: true; enabled: !root.controlsBusy; onClicked: root.runAction(["service", root.serviceState.active === "active" ? "stop" : "start"], "Updating model service…", "Model service updated.") }
              }
              Caption { text: root.serviceError || "Service: " + (root.serviceState.active || "checking") + " · " + (root.serviceState.state || "") }
              PanelSeparator { Layout.fillWidth: true; foreground: Color.popups.text }
              SectionTitle { text: "From your terminal" }
              Caption { text: "The same controls are available without opening this panel. Select and copy a command below." }
              Controls.TextArea {
                Layout.fillWidth: true
                readOnly: true; selectByMouse: true; wrapMode: TextEdit.Wrap
                text: "badi status\nbadi pause\nbadi resume\nbadi app list\nbadi apps blocklist\nbadi site https://example.com off\nbadi autostart off\nbadi doctor\nbadi settings --json"
                color: Color.accent; font.family: Style.font.family; font.pixelSize: Style.font.body
                background: Rectangle { color: Color.popups.background }
                Accessible.name: "Badi terminal command reference"
              }
              Caption { text: "Use badi --help for all commands, or badictl --help for advanced controls. Settings updates reject concurrent edits instead of overwriting them." }
            }
          }
        }
        PanelSeparator { Layout.fillWidth: true; foreground: Color.popups.text }
        RowLayout {
          Layout.fillWidth: true
          Caption {
            text: root.actionMessage || client.message || "Right-click the Badi bar icon to pause or resume."
            color: root.actionMessage ? (root.actionFailed ? Color.urgent : Color.popups.text) : client.messageTone === "danger" ? Color.urgent : Color.popups.text
          }
          ActionButton { text: "Refresh"; focusable: true; enabled: !root.controlsBusy; onClicked: { root.actionMessage = ""; client.refresh(); root.probeService() } }
        }
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
