import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

FocusScope {
    id: root
    focus: true

    required property var client
    signal closeRequested()

    property bool confirmMemoryClear: false
    property double wallClockMs: Date.now()
    readonly property bool snapshotFresh: client.lastRefreshed.getTime() > 0
        && wallClockMs - client.lastRefreshed.getTime() <= 30000

    function resetTransientState() {
        confirmMemoryClear = false;
    }

    function brokerStatusText() {
        if (client.loading) return "Checking status";
        if (client.errorMessage.length > 0 || !client.brokerReachable)
            return "Status unavailable";
        return snapshotFresh ? "Broker reached" : "Snapshot stale";
    }

    function brokerStatusTone() {
        if (client.loading) return "info";
        if (client.errorMessage.length > 0 || !client.brokerReachable) return "danger";
        return snapshotFresh ? "positive" : "warning";
    }

    function titleCase(value) {
        if (typeof value !== "string" || value.length === 0 || value === "unknown")
            return "Not reported";
        return value.replace(/_/g, " ").replace(/\b\w/g, function(character) {
            return character.toUpperCase();
        });
    }

    function byteLabel(bytes) {
        if (!Number.isSafeInteger(bytes) || bytes < 0) return "Not reported";
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KiB";
        return (bytes / (1024 * 1024)).toFixed(1) + " MiB";
    }

    function modelTone() {
        if (client.modelReady) return "positive";
        if (client.modelStatus === "no_fit") return "danger";
        if (client.modelStatus === "candidate") return "warning";
        return "neutral";
    }

    Shortcut {
        sequence: "Ctrl+R"
        onActivated: client.refresh()
    }

    Shortcut {
        sequence: "Ctrl+P"
        enabled: client.canTogglePause
        onActivated: client.toggleEffectivePause()
    }

    Shortcut {
        sequence: "Escape"
        onActivated: root.closeRequested()
    }

    Timer {
        interval: 1000
        repeat: true
        running: root.visible
        onTriggered: root.wallClockMs = Date.now()
    }

    Rectangle {
        anchors.fill: parent
        color: BadiTheme.window
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 128
            color: BadiTheme.window

            ColumnLayout {
                anchors.fill: parent
                anchors.leftMargin: 20
                anchors.rightMargin: 20
                anchors.topMargin: 16
                anchors.bottomMargin: 14
                spacing: 10

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2

                        Text {
                            text: "Badi"
                            color: BadiTheme.text
                            font.pixelSize: 24
                            font.weight: Font.DemiBold
                        }

                        Text {
                            text: "Local co-writing controls"
                            color: BadiTheme.textMuted
                            font.pixelSize: 12
                        }
                    }

                    ActionButton {
                        id: pauseButton
                        text: client.brokerPaused ? "Resume" : "Pause"
                        tone: client.brokerPaused ? "accent" : "neutral"
                        enabled: client.canTogglePause
                        Accessible.description: "Pause Badi fail-closed, or resume only when control-plane authority is healthy. Shortcut Control P."
                        onClicked: client.toggleEffectivePause()
                        ToolTip.visible: hovered
                        ToolTip.text: client.brokerPaused ? "Resume Badi (Ctrl+P)" : "Pause Badi (Ctrl+P)"
                    }

                    ActionButton {
                        id: refreshButton
                        text: client.loading ? "Refreshing…" : "Refresh"
                        enabled: !client.busy
                        focus: true
                        Accessible.description: "Refresh content-free Badi status. Shortcut Control R."
                        onClicked: client.refresh()
                        ToolTip.visible: hovered
                        ToolTip.text: "Refresh status (Ctrl+R)"
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    StatusPill {
                        text: root.brokerStatusText()
                        tone: root.brokerStatusTone()
                    }

                    StatusPill {
                        text: !client.controlPlaneDegradedReported
                            || !client.settingsPausedReported || !client.brokerPausedReported
                            ? "State unknown"
                            : client.controlPlaneDegraded
                            ? "Degraded"
                            : !client.pauseStateConsistent
                                ? "State mismatch"
                                : client.brokerPaused ? "Paused" : "Active"
                        tone: !client.controlPlaneDegradedReported
                            || !client.settingsPausedReported || !client.brokerPausedReported
                            ? "neutral"
                            : client.controlPlaneDegraded
                            ? "warning"
                            : !client.pauseStateConsistent || client.brokerPaused
                                ? "warning" : "positive"
                        visible: client.brokerReachable
                    }

                    Item { Layout.fillWidth: true }

                    Text {
                        visible: client.lastRefreshed.getTime() > 0
                        text: "Snapshot " + client.lastRefreshed.toLocaleTimeString(
                            Qt.locale(), "HH:mm:ss")
                        color: root.snapshotFresh ? BadiTheme.textDim : BadiTheme.warning
                        font.pixelSize: 11
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: BadiTheme.border
        }

        ScrollView {
            id: scrollView
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentWidth: availableWidth
            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

            ColumnLayout {
                width: scrollView.availableWidth
                spacing: 14

                Item { implicitHeight: 4 }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    implicitHeight: errorLayout.implicitHeight + 24
                    radius: BadiTheme.radiusMedium
                    color: BadiTheme.dangerSurface
                    border.width: 1
                    border.color: BadiTheme.withAlpha(BadiTheme.danger, 0.5)
                    visible: client.errorMessage.length > 0
                    Accessible.role: Accessible.AlertMessage
                    Accessible.name: "Badi status is unavailable. " + client.errorMessage

                    RowLayout {
                        id: errorLayout
                        anchors.fill: parent
                        anchors.margins: 12
                        spacing: 12

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            Text {
                                text: "Badi status is unavailable"
                                color: BadiTheme.danger
                                font.pixelSize: 13
                                font.weight: Font.DemiBold
                            }

                            Text {
                                Layout.fillWidth: true
                                text: client.errorMessage
                                color: BadiTheme.textMuted
                                font.pixelSize: 11
                                wrapMode: Text.WordWrap
                            }
                        }

                        ActionButton {
                            text: "Retry"
                            enabled: !client.busy
                            onClicked: client.refresh()
                        }
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    implicitHeight: noticeText.implicitHeight + 24
                    radius: BadiTheme.radiusMedium
                    color: BadiTheme.toneBackground(client.noticeTone)
                    border.width: 1
                    border.color: BadiTheme.withAlpha(BadiTheme.toneForeground(client.noticeTone), 0.4)
                    visible: client.noticeMessage.length > 0
                    Accessible.role: Accessible.AlertMessage
                    Accessible.name: client.noticeMessage

                    Text {
                        id: noticeText
                        anchors.fill: parent
                        anchors.margins: 12
                        text: client.noticeMessage
                        color: BadiTheme.toneForeground(client.noticeTone)
                        font.pixelSize: 12
                        wrapMode: Text.WordWrap
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    implicitHeight: degradedLayout.implicitHeight + 24
                    radius: BadiTheme.radiusMedium
                    color: BadiTheme.warningSurface
                    border.width: 1
                    border.color: BadiTheme.withAlpha(BadiTheme.warning, 0.55)
                    visible: client.brokerReachable
                        && client.controlPlaneDegradedReported
                        && client.controlPlaneDegraded
                    Accessible.role: Accessible.AlertMessage
                    Accessible.name: "Control plane degraded. Badi is fail-closed and settings changes are disabled."

                    ColumnLayout {
                        id: degradedLayout
                        anchors.fill: parent
                        anchors.margins: 12
                        spacing: 3

                        Text {
                            text: "Control plane degraded — changes disabled"
                            color: BadiTheme.warning
                            font.pixelSize: 13
                            font.weight: Font.DemiBold
                        }

                        Text {
                            Layout.fillWidth: true
                            text: "Badi remains fail-closed until coherent authority is restored or the broker is restarted."
                            color: BadiTheme.textMuted
                            font.pixelSize: 11
                            wrapMode: Text.WordWrap
                        }
                    }
                }

                SectionCard {
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    title: "Control"
                    subtitle: "Pause and target permissions live in one strict, revision-checked settings document."

                    InfoRow {
                        title: "Persisted pause"
                        detail: !client.controlPlaneDegradedReported
                            ? "The broker did not report control-plane condition; mutation is disabled."
                            : client.pauseStateConsistent
                            ? client.controlPlaneDegraded
                                ? "The settings revision is coherent, but the broker is fail-closed after a control-plane error."
                                : client.brokerPaused !== client.settingsPaused
                                    ? "The snapshot is coherent; a separate runtime pause is currently effective."
                                    : "The broker and badi.settings.v1 report one coherent authority snapshot."
                            : "Broker and settings revisions are missing or disagree; mutation is disabled."
                        value: !client.settingsPausedReported
                            ? "Not reported" : client.settingsPaused ? "Paused" : "Active"
                        tone: client.controlPlaneDegradedReported
                            && client.pauseStateConsistent
                            ? client.controlPlaneDegraded || client.settingsPaused
                                ? "warning" : "positive"
                            : "warning"
                    }

                    InfoRow {
                        title: "Current provider"
                        detail: "The provider is selected by the broker, never by this view."
                        value: root.titleCase(client.brokerProvider)
                        tone: client.brokerProvider === "local_model" ? "info" : "neutral"
                    }
                }

                SectionCard {
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    title: "Privacy boundary"
                    subtitle: "These are independently reported capabilities. Local inference does not widen context access."

                    InfoRow {
                        title: "Active-field context"
                        detail: "Only the supported focused field; acquisition still depends on adapter policy."
                        value: client.maxBeforeChars >= 0 && client.maxAfterChars >= 0
                            ? client.maxBeforeChars + " before · " + client.maxAfterChars + " after"
                            : "Not reported"
                        tone: "info"
                    }

                    InfoRow {
                        title: "Clipboard"
                        detail: "Clipboard access is a separate permission and is not implied by app approval."
                        value: !client.clipboardReported ? "Not reported" : client.clipboardAllowed ? "Allowed" : "Off"
                        tone: !client.clipboardReported ? "neutral" : client.clipboardAllowed ? "warning" : "positive"
                    }

                    InfoRow {
                        title: "Screen and window text"
                        detail: "No screenshot or compositor-wide text capture."
                        value: !client.screenReported ? "Not reported" : client.screenAllowed ? "Allowed" : "Off"
                        tone: !client.screenReported ? "neutral" : client.screenAllowed ? "warning" : "positive"
                    }

                    InfoRow {
                        title: "Network inference"
                        detail: "A local provider never silently falls back to a remote service."
                        value: !client.networkReported ? "Not reported" : client.networkAllowed ? "Allowed" : "Off"
                        tone: !client.networkReported ? "neutral" : client.networkAllowed ? "warning" : "positive"
                    }
                }

                SectionCard {
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    title: "Development browser origin"
                    subtitle: "This milestone edits one exact Chromium-origin subject. It is not a generic app or site manager."

                    InfoRow {
                        title: "Unlisted identities"
                        detail: "badi.settings.v1 has no permissive default or wildcard rule."
                        value: client.settingsDocumentValid ? "Blocked" : "Not reported"
                        tone: client.settingsDocumentValid ? "positive" : "neutral"
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 1
                        color: BadiTheme.border
                    }

                    InfoRow {
                        title: "Chromium exact-document gate"
                        detail: "Declared by the development manifest. Chromium—not Badi policy—owns effective site access."
                        value: root.titleCase(client.browserPermission)
                        tone: client.browserPermission.indexOf("static") >= 0 ? "warning" : "neutral"
                    }

                    InfoRow {
                        title: "Badi identity gate"
                        detail: "Resolved independently by the broker from strict browser-origin subjects."
                        value: root.titleCase(client.badiPolicy)
                        tone: client.badiPolicy === "exact_origin_subjects" ? "positive" : "warning"
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Text {
                            Layout.fillWidth: true
                            text: "localhost:4173 suggestion bundle"
                            color: BadiTheme.text
                            font.pixelSize: 13
                            font.weight: Font.Medium
                        }

                        Text {
                            Layout.fillWidth: true
                            text: !client.memoryStoreAvailableReported
                                ? "Aggregate-store condition is not reported. Per-target changes remain disabled; refresh status before using controls."
                                : !client.memoryStoreAvailable
                                ? "The aggregate store requires repair. Per-target changes are frozen to preserve privacy consistency; "
                                    + (client.canTogglePause
                                        ? "use Pause now, then clear or repair Memory."
                                        : "clear or repair Memory before changing policy.")
                                : client.fixtureCapacityReached
                                ? "The 64-subject settings limit is reached. Remove a subject before adding localhost:4173."
                                : "Explicitly allow all three required gates or block them all. Partial external settings are never interpreted as a toggle."
                            color: BadiTheme.textDim
                            font.pixelSize: 11
                            wrapMode: Text.WordWrap
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8

                            Item { Layout.fillWidth: true }

                            ActionButton {
                                text: "Block all"
                                tone: "danger"
                                enabled: client.canMutateSubjects
                                    && client.fixtureAnySuggestionPermissionAllowed
                                Accessible.description: !client.memoryStoreAvailableReported
                                    ? "Unavailable until status refresh reports aggregate-store condition."
                                    : !client.memoryStoreAvailable
                                    ? "Unavailable until the aggregate store is repaired."
                                        + (client.canTogglePause
                                            ? " Use Pause for an immediate global stop."
                                            : "")
                                    : "Block context, display, suggestions, and aggregate learning for localhost:4173."
                                onClicked: client.setFixtureSuggestionBundle(false)
                            }

                            ActionButton {
                                text: "Allow bundle"
                                tone: "accent"
                                enabled: client.canMutateSubjects
                                    && !client.fixtureSuggestionBundleAllowed
                                    && client.fixtureCanBeAdded
                                Accessible.description: !client.memoryStoreAvailableReported
                                    ? "Unavailable until status refresh reports aggregate-store condition."
                                    : !client.memoryStoreAvailable
                                    ? "Unavailable until the aggregate store is repaired."
                                    : client.fixtureCapacityReached
                                    ? "The 64-subject settings limit is reached."
                                    : "Allow context, display, and suggestions for localhost:4173."
                                onClicked: client.setFixtureSuggestionBundle(true)
                            }
                        }
                    }

                    InfoRow {
                        title: "Origin permission decisions"
                        detail: "The bundle editor changes these three schema gates together; partial external configurations remain visible."
                        value: "Context " + root.titleCase(client.fixtureContextDecision)
                            + " · Display " + root.titleCase(client.fixtureDisplayDecision)
                            + " · Suggest " + root.titleCase(client.fixtureSuggestDecision)
                        tone: client.fixtureSuggestionBundleAllowed ? "positive" : "warning"
                    }

                    InfoRow {
                        title: "Declared development document"
                        detail: "The Badi subject is origin-wide; the separate extension manifest currently narrows injection to this document."
                        value: client.supportedScope.length > 0
                            ? client.supportedScope : "Not reported"
                        tone: "info"
                    }

                    InfoRow {
                        title: "Native application rules"
                        detail: "No trustworthy native-app identity source is implemented in this milestone."
                        value: "Unsupported"
                        tone: "neutral"
                    }

                    InfoRow {
                        title: "Obsidian"
                        detail: "A target-native CodeMirror adapter and headed evidence do not exist yet."
                        value: "Unsupported"
                        tone: "neutral"
                    }

                    InfoRow {
                        title: "Terminal"
                        detail: "Ambient terminal context and synthetic insertion remain outside Badi's supported boundary."
                        value: "Unsupported"
                        tone: "neutral"
                    }
                }

                SectionCard {
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    title: "Local writing model"
                    subtitle: "Hardware advice is not activation. A candidate must be installed, verified, and pass Badi's runtime quality gate."

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 3

                            Text {
                                Layout.fillWidth: true
                                text: client.modelRepository
                                color: BadiTheme.text
                                font.pixelSize: 14
                                font.weight: Font.DemiBold
                                wrapMode: Text.WrapAnywhere
                            }

                            Text {
                                Layout.fillWidth: true
                                text: [
                                    root.titleCase(client.modelTier),
                                    client.modelQuantization,
                                    client.modelLicense
                                ].join(" · ")
                                color: BadiTheme.textMuted
                                font.pixelSize: 11
                                wrapMode: Text.WordWrap
                            }
                        }

                        StatusPill {
                            text: client.modelReady
                                ? "Runtime ready"
                                : client.modelStatus === "candidate"
                                    ? "Candidate only"
                                    : root.titleCase(client.modelStatus)
                            tone: root.modelTone()
                        }
                    }

                    InfoRow {
                        title: "Artifact"
                        detail: "A filename is metadata, not proof that the file exists or matches its digest."
                        value: client.modelFilename.length > 0 ? client.modelFilename : "Not reported"
                    }

                    InfoRow {
                        title: "Installed"
                        detail: "Badi does not download model weights from this control center."
                        value: !client.modelInstalledReported ? "Not reported" : client.modelInstalled ? "Yes" : "No"
                        tone: !client.modelInstalledReported ? "neutral" : client.modelInstalled ? "positive" : "warning"
                    }

                    InfoRow {
                        title: "Configured"
                        detail: "A verified artifact must be deliberately bound to the broker; recommendation alone never configures it."
                        value: !client.modelConfiguredReported ? "Not reported" : client.modelConfigured ? "Yes" : "No"
                        tone: !client.modelConfiguredReported ? "neutral" : client.modelConfigured ? "positive" : "warning"
                    }

                    InfoRow {
                        title: "Quality gate"
                        detail: client.modelReason.length > 0
                            ? root.titleCase(client.modelReason)
                            : "Latency, cancellation, memory, usefulness, and privacy must all pass."
                        value: !client.modelReadyReported ? "Not reported" : client.modelReady ? "Passed" : "Not passed"
                        tone: !client.modelReadyReported ? "neutral" : client.modelReady ? "positive" : "warning"
                    }
                }

                SectionCard {
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    title: "Local interaction aggregates"
                    subtitle: "Optional per-origin daily counters for broker-emitted suggestions and requested actions. They are text-free metadata, not adaptive writing memory or proof of display/application."

                    InfoRow {
                        title: "Adaptive writing memory"
                        detail: "Badi does not currently retain text, phrases, style features, or cross-app writing context."
                        value: !client.learningAvailableReported
                            ? "Not reported"
                            : client.learningAvailable
                                ? "Available"
                                : root.titleCase(client.adaptiveWritingMemory)
                        tone: client.learningAvailable ? "warning" : "neutral"
                    }

                    InfoRow {
                        title: "Aggregate subsystem"
                        detail: "Global text-free status. Persisted records contain origin, provider, UTC day, and counts."
                        value: root.titleCase(client.outcomeAggregates)
                            + (client.reportedMaxRetentionDays >= 0
                                ? " · up to " + client.reportedMaxRetentionDays + " days" : "")
                        tone: client.outcomeAggregates === "persisted"
                            || client.outcomeAggregates === "memory_only" ? "info" : "neutral"
                    }

                    SettingRow {
                        title: "Collect localhost:4173 outcome aggregates"
                        detail: "Enables memory-only per-origin daily counters. Disk retention is a separate explicit choice below. No prose is stored."
                        checked: client.fixtureOutcomeAggregatesAllowed
                        controlEnabled: client.canMutateSubjects
                            && client.fixtureSuggestionBundleAllowed
                            && (client.fixtureOutcomeAggregatesAllowed
                                || client.memoryAvailable)
                        onRequested: checked => client.setOutcomeAggregates(checked)
                    }

                    InfoRow {
                        title: "Origin aggregate permission"
                        detail: "The schema's learn gate currently authorizes counters only; bounded retention controls disk persistence. Accepted counters mean commit requested, not confirmed applied."
                        value: "Learn " + root.titleCase(client.fixtureLearnDecision)
                            + " · Retention "
                            + (!client.fixtureOutcomeAggregatesAllowed
                                ? "Off"
                                : client.fixtureRetentionDays >= 0
                                ? client.fixtureRetentionDays + " days"
                                : "Memory only")
                        tone: client.fixtureOutcomeAggregatesAllowed ? "info" : "neutral"
                    }

                    InfoRow {
                        title: "Aggregate records"
                        detail: "Number of daily origin/provider records; not learned phrases and not confirmed delivery outcomes."
                        value: client.memoryRecords >= 0 ? String(client.memoryRecords) : "Not reported"
                        tone: "info"
                    }

                    InfoRow {
                        title: "Storage"
                        detail: "Local XDG state owned by Badi. The control center never reads the store directly."
                        value: root.byteLabel(client.memoryBytes)
                    }

                    InfoRow {
                        title: "Recorder health"
                        detail: client.memoryStoreAvailable
                            ? "Write failures and queue drops are text-free cumulative counters for this broker process. Any nonzero value marks recorder integrity degraded since start."
                            : "The broker preserved unreadable aggregate state and disabled recording. Use the explicit clear action below to recover; no file is silently replaced."
                        value: !client.memoryAvailableReported
                            ? "Not reported"
                            : client.memoryAvailable
                                ? "Healthy"
                                : !client.memoryStoreAvailable
                                    ? "Store unavailable · clear required"
                                    : "Unavailable · " + Math.max(0, client.memoryWriteFailures)
                                    + " write failures · "
                                    + Math.max(0, client.memoryDroppedSignals) + " drops"
                        tone: client.memoryAvailable ? "positive" : "warning"
                    }

                    Text {
                        Layout.fillWidth: true
                        text: "RETENTION"
                        color: BadiTheme.textDim
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.letterSpacing: 0.8
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Repeater {
                            model: [0, 7, 30, 90]

                            ActionButton {
                                required property int modelData
                                Layout.fillWidth: true
                                text: (selected ? "✓ " : "")
                                    + (modelData === 0 ? "Memory only" : modelData + " days")
                                selected: client.fixtureOutcomeAggregatesAllowed
                                    && (modelData === 0
                                        ? !client.fixtureOutcomeAggregatesPersisted
                                        : client.fixtureRetentionDays === modelData)
                                selectionControl: true
                                enabled: client.canMutateSubjects
                                    && client.fixtureOutcomeAggregatesAllowed
                                    && (modelData === 0 || client.memoryAvailable)
                                Accessible.description: modelData === 0
                                    ? "Keep localhost:4173 text-free aggregates in memory only."
                                    : "Retain localhost:4173 text-free origin and interaction metadata for " + modelData + " days."
                                onClicked: client.setRetentionDays(modelData)
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 1
                        color: BadiTheme.border
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            Text {
                                text: root.confirmMemoryClear
                                    ? "Clear all outcome aggregates?" : "Delete outcome aggregates"
                                color: root.confirmMemoryClear ? BadiTheme.danger : BadiTheme.text
                                font.pixelSize: 13
                                font.weight: Font.Medium
                            }

                            Text {
                                Layout.fillWidth: true
                                text: root.confirmMemoryClear
                                    ? "This removes Badi's local text-free origin/day counter records. It does not change origin permissions."
                                    : "Requires a second confirmation and uses badictl's validated deletion path."
                                color: BadiTheme.textDim
                                font.pixelSize: 11
                                wrapMode: Text.WordWrap
                            }
                        }

                        ActionButton {
                            visible: root.confirmMemoryClear
                            text: "Confirm clear"
                            tone: "danger"
                            enabled: client.memoryCommandAvailableReported
                                && client.memoryCommandAvailable && !client.busy
                            onClicked: {
                                root.confirmMemoryClear = false;
                                client.clearMemory();
                            }
                        }

                        ActionButton {
                            text: root.confirmMemoryClear ? "Cancel" : "Clear aggregates…"
                            tone: root.confirmMemoryClear ? "neutral" : "danger"
                            enabled: client.memoryCommandAvailableReported
                                && client.memoryCommandAvailable && !client.busy
                            onClicked: {
                                root.confirmMemoryClear = !root.confirmMemoryClear;
                            }
                        }
                    }
                }

                SectionCard {
                    Layout.leftMargin: 20
                    Layout.rightMargin: 20
                    title: "Diagnostics"
                    subtitle: "Content-free operational facts. Evidence labels describe what was actually reproduced."

                    InfoRow {
                        title: "Broker sessions"
                        value: client.brokerSessions >= 0 ? String(client.brokerSessions) : "Not reported"
                    }

                    InfoRow {
                        title: "Authority epoch"
                        detail: "A policy or pause transition advances this receiver-local authority fence."
                        value: client.authorityEpoch >= 0
                            ? String(client.authorityEpoch) : "Not reported"
                    }

                    InfoRow {
                        title: "Settings revision"
                        detail: "Every replacement must compare against this revision and advance it exactly once."
                        value: client.settingsRevision >= 0
                            ? String(client.settingsRevision) : "Not reported"
                    }

                    InfoRow {
                        title: "Socket boundary"
                        detail: "Private Unix-socket mode reported by the broker."
                        value: client.socketMode
                        tone: client.socketMode === "0600" ? "positive" : "warning"
                    }

                    InfoRow {
                        title: "Frame limit"
                        value: client.maxFrameBytes >= 0 ? client.maxFrameBytes + " bytes" : "Not reported"
                    }

                    InfoRow {
                        title: "Capability evidence"
                        detail: client.evidenceCommit.length > 0
                            ? "Recorded source " + client.evidenceCommit.slice(0, 12) + ". Historical evidence is not proof of this tree."
                            : "No immutable source identity was reported."
                        value: root.titleCase(client.evidenceClass)
                        tone: client.evidenceClass === "current" ? "positive" : "warning"
                    }

                    InfoRow {
                        title: "Last refreshed"
                        detail: "Refreshes are explicit; the UI does not continuously probe hardware."
                        value: client.lastRefreshed.getTime() > 0
                            ? client.lastRefreshed.toLocaleTimeString(Qt.locale(), "HH:mm:ss")
                            : "Never"
                    }
                }

                Text {
                    Layout.fillWidth: true
                    Layout.leftMargin: 24
                    Layout.rightMargin: 24
                    Layout.bottomMargin: 24
                    text: "Badi stays silent when target identity, permission, policy, or runtime readiness cannot be proved."
                    color: BadiTheme.textDim
                    font.pixelSize: 11
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                }
            }
        }
    }
}
