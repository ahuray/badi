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
  readonly property bool anyAppEnabled: appEnabled("omawrite") || appEnabled("com.github.xournalpp.xournalpp") || appEnabled("obsidian") || appEnabled("bash")
  readonly property bool ready: client.brokerReachable && !client.brokerPaused && !client.controlPlaneDegraded && anyAppEnabled
  readonly property bool paused: client.brokerReachable && client.brokerPaused
  readonly property string statusLabel: !client.brokerReachable
    ? (serviceState.active === "active" ? "Model starting or unavailable" : "Model offline")
    : client.controlPlaneDegraded ? "Settings need attention" : paused ? "Predictions paused"
    : !anyAppEnabled ? "Applications disabled" : "Model ready"
  readonly property var activity: client.overview.desktop ? client.overview.desktop.activity || ({}) : ({})
  property string selectedTrial: ""
  property int page: 0
  property var serviceState: ({})
  property string serviceError: ""
  property string actionMessage: ""
  property string actionSuccess: ""
  property bool actionFailed: false
  property bool actionTimedOut: false
  readonly property bool desktopSelected: !selectedTrial || selectedTrial === "desktop"
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

  function appEnabled(appId) {
    var subjects = client.settings.subjects || []
    for (var i = 0; i < subjects.length; i++) {
      var entry = subjects[i]
      if (entry.identity.kind === "linux_app" && entry.identity.adapter === appAdapter(appId) && entry.identity.app_id === appId)
        return entry.permissions.context_read === "allow" && entry.permissions.suggest === "allow" && entry.permissions.display === "allow"
    }
    return false
  }

  function appAdapter(appId) {
    return appId === "obsidian" ? "obsidian" : appId === "bash" ? "shell" : "fcitx"
  }

  function toggleApp(appId) {
    if (!client.canMutateSettings || action.running) return
    actionMessage = ""
    var enabled = !appEnabled(appId)
    var document = client.cloneSettings()
    var identity = {kind: "linux_app", adapter: appAdapter(appId), app_id: appId}
    var index = -1
    for (var i = 0; i < document.subjects.length; i++) {
      var item = document.subjects[i].identity
      if (item.kind === "linux_app" && item.adapter === appAdapter(appId) && item.app_id === appId) index = i
    }
    var decision = enabled ? "allow" : "block"
    var subject = {identity: identity, permissions: {context_read: decision, display: decision,
      suggest: decision, learn: "block", retention: {mode: "none"}}}
    if (index < 0) document.subjects.push(subject)
    else document.subjects[index] = subject
    document.subjects.sort(function(a, b) { return client.compareIdentities(a.identity, b.identity) })
    client.replaceSettings(document, enabled ? "App enabled. Predictions are available in supported text fields." : "App blocked. Context reading and predictions are disabled.")
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
    cliPrefix: root.selectedTrial ? [root.helper, "ctl", "--trial", root.selectedTrial] : [root.helper, "ctl"]
    mutationPrefix: [root.helper, "ctl", "--trial", overview.desktop ? overview.desktop.id : ""]
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
              BodyText { text: "In Omawrite or a Xournal++ text cell, press Tab at the end of a phrase to request words; press Tab again to insert them. Web and Obsidian integrations suggest automatically: Tab takes one word, Ctrl/Command+Right takes all." }
              RowLayout {
                ActionButton { text: "Open Omawrite"; bordered: true; focusable: true; enabled: !root.controlsBusy; onClicked: root.runAction(["launch", "omawrite"], "Opening Omawrite…", "Omawrite opened. Type a phrase and press Tab.") }
                ActionButton { text: "Open Xournal++"; bordered: true; focusable: true; enabled: !root.controlsBusy; onClicked: root.runAction(["launch", "xournalpp"], "Opening Xournal++…", "Xournal++ opened. Select the Text tool and click the page.") }
              }
              Caption { text: "Example: Please find attached the\nEscape dismisses. Suggestions last five seconds and clear when text or focus changes." }
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
              SectionTitle { text: "Where Badi can write" }
              BodyText { text: "Allow an app to use its connected writing integration. Blocking it disables context reading and predictions." }
              Repeater {
                model: [{name: "Omawrite", appId: "omawrite", detail: "Markdown editor · Qt 6"},
                  {name: "Xournal++", appId: "com.github.xournalpp.xournalpp", detail: "Text tool · GTK 3"},
                  {name: "Obsidian", appId: "obsidian", detail: "Badi plugin · Tab accepts a word · Ctrl/Command+Right accepts all"},
                  {name: "Bash", appId: "bash", detail: "Ctrl-X Tab requests / accepts · Tab completes commands"}]
                ColumnLayout {
                  required property var modelData
                  Layout.fillWidth: true
                  RowLayout {
                    Layout.fillWidth: true
                    BodyText { text: modelData.name; font.bold: true }
                    ActionButton {
                      text: !client.brokerReachable ? "Unavailable" : root.appEnabled(modelData.appId) ? "Enabled · Block" : "Blocked · Enable"
                      bordered: true; focusable: true; enabled: client.canMutateSettings && !action.running
                      onClicked: root.toggleApp(modelData.appId)
                      Accessible.name: modelData.name + " predictions: " + (root.appEnabled(modelData.appId) ? "enabled; activate to block" : "blocked; activate to enable")
                    }
                  }
                  Caption { text: modelData.detail }
                }
              }
              PanelSeparator { Layout.fillWidth: true; foreground: Color.popups.text }
              SectionTitle { text: "Connect your editors" }
              BodyText { text: "Obsidian needs the Badi vault plugin. Bash needs the shell hook. Browser text fields need the Badi extension and permission for each site." }
              Caption { text: "Run badi doctor or badi debug watch to check activity. Rich website editors and native spelling replacement are still in development." }
              Controls.ComboBox {
                Layout.fillWidth: true
                visible: !!client.overview.desktop && client.overview.desktop.sessions.length > 1
                model: client.overview.desktop ? client.overview.desktop.sessions : []
                textRole: "label"
                currentIndex: {
                  if (!client.overview.desktop) return -1
                  for (var i = 0; i < model.length; i++) if (model[i].id === client.overview.desktop.id) return i
                  return -1
                }
                onActivated: (index) => { root.selectedTrial = model[index].id; root.actionMessage = ""; client.refresh() }
                Accessible.name: "Writing session controlled by this panel"
              }
              Caption { visible: !root.desktopSelected; text: "These prediction controls apply to the selected trial. System controls always manage the desktop model." }
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
                text: "badi status\nbadi pause\nbadi resume\nbadi app omawrite off\nbadi autostart off\nbadi service stop\nbadi doctor\nbadi logs\nbadi settings --json"
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
