import QtQuick
import Quickshell
import Quickshell.Io

Scope {
    id: root

    readonly property string fixtureOrigin: "http://localhost:4173"

    property var overview: ({})
    property string errorMessage: ""
    property string noticeMessage: ""
    property string noticeTone: "neutral"
    property date lastRefreshed: new Date(0)
    property bool overviewTimedOut: false
    property bool mutationTimedOut: false

    readonly property bool loading: overviewProcess.running
    readonly property bool mutating: mutationProcess.running
    readonly property bool busy: loading || mutating

    readonly property var broker: objectValue(overview, "broker")
    readonly property var settings: objectValue(overview, "settings")
    readonly property var privacy: objectValue(overview, "privacy")
    readonly property var support: objectValue(overview, "support")
    readonly property var models: objectValue(overview, "models")
    readonly property var writingModel: objectValue(models, "writing")
    readonly property var writingAdvice: objectValue(writingModel, "advice")

    readonly property bool brokerReachable: boolValue(broker, "reachable", false)
    readonly property bool brokerPaused: boolValue(broker, "paused", false)
    readonly property bool brokerPausedReported: hasBoolean(broker, "paused")
    readonly property string brokerProvider: stringValue(broker, "provider", "unknown")
    readonly property double brokerSessions: integerValue(broker, "sessions", -1)
    readonly property double authorityEpoch: integerValue(broker, "authority_epoch", -1)
    readonly property double brokerSettingsRevision: integerValue(
        broker, "settings_revision", -1)
    readonly property bool controlPlaneDegraded: boolValue(
        broker, "control_plane_degraded", true)
    readonly property bool controlPlaneDegradedReported: hasBoolean(
        broker, "control_plane_degraded")
    readonly property string socketMode: stringValue(broker, "socket_mode", "unknown")
    readonly property int maxFrameBytes: integerValue(broker, "max_frame_bytes", -1)

    readonly property bool settingsDocumentValid: isSettingsDocument(settings)
    readonly property double settingsRevision: settingsDocumentValid ? settings.revision : -1
    readonly property bool settingsPaused: settingsDocumentValid ? settings.paused : true
    readonly property bool settingsPausedReported: settingsDocumentValid
    readonly property bool snapshotCoherent: brokerSettingsRevision >= 0
        && settingsRevision >= 0 && brokerSettingsRevision === settingsRevision
    readonly property bool pauseStateConsistent: snapshotCoherent && brokerPausedReported
        && settingsPausedReported
    readonly property bool canMutateSettings: brokerReachable && settingsDocumentValid
        && snapshotCoherent && controlPlaneDegradedReported && !controlPlaneDegraded
        && settingsRevision >= 0 && !busy
    readonly property bool canRuntimeControl: brokerReachable && brokerPausedReported
        && !busy
    readonly property bool canTogglePause: brokerPaused
        ? settingsPaused
            ? canMutateSettings
            : canRuntimeControl && !controlPlaneDegraded
        : canMutateSettings || canRuntimeControl
    readonly property int fixtureSubjectIndex: settingsDocumentValid
        ? findFixtureSubject(settings.subjects) : -1
    readonly property var fixtureSubject: fixtureSubjectIndex >= 0
        ? settings.subjects[fixtureSubjectIndex] : ({})
    readonly property var fixturePermissions: objectValue(fixtureSubject, "permissions")
    readonly property string fixtureContextDecision: fixtureDecision("context_read")
    readonly property string fixtureDisplayDecision: fixtureDecision("display")
    readonly property string fixtureSuggestDecision: fixtureDecision("suggest")
    readonly property string fixtureLearnDecision: fixtureDecision("learn")
    readonly property bool fixtureSuggestionBundleAllowed:
        fixtureSubjectIndex >= 0
        && fixturePermissions.context_read === "allow"
        && fixturePermissions.display === "allow"
        && fixturePermissions.suggest === "allow"
    readonly property bool fixtureAnySuggestionPermissionAllowed:
        fixtureSubjectIndex >= 0
        && (fixturePermissions.context_read === "allow"
            || fixturePermissions.display === "allow"
            || fixturePermissions.suggest === "allow")
    readonly property bool fixtureCapacityReached: settingsDocumentValid
        && fixtureSubjectIndex < 0 && settings.subjects.length >= 64
    readonly property bool fixtureCanBeAdded: settingsDocumentValid
        && !fixtureCapacityReached
    readonly property bool fixtureOutcomeAggregatesAllowed:
        fixtureSuggestionBundleAllowed
        && fixturePermissions.learn === "allow"
    readonly property bool fixtureOutcomeAggregatesPersisted:
        fixtureOutcomeAggregatesAllowed
        && isBoundedRetention(fixturePermissions.retention)
    readonly property int fixtureRetentionDays: fixtureOutcomeAggregatesPersisted
        ? integerValue(fixturePermissions.retention, "days", -1) : -1

    readonly property int maxBeforeChars: integerValue(privacy, "max_before_chars", -1)
    readonly property int maxAfterChars: integerValue(privacy, "max_after_chars", -1)
    readonly property bool clipboardAllowed: boolValue(privacy, "clipboard", false)
    readonly property bool clipboardReported: hasBoolean(privacy, "clipboard")
    readonly property bool screenAllowed: boolValue(privacy, "screen", false)
    readonly property bool screenReported: hasBoolean(privacy, "screen")
    readonly property bool networkAllowed: boolValue(privacy, "network", false)
    readonly property bool networkReported: hasBoolean(privacy, "network")
    readonly property string adaptiveWritingMemory: stringValue(
        privacy, "adaptive_writing_memory", "unknown")
    readonly property string outcomeAggregates: stringValue(
        privacy, "outcome_aggregates", "unknown")
    readonly property string aggregateSemantics: stringValue(
        privacy, "aggregate_semantics", "unknown")
    readonly property string storedMetadata: stringValue(
        privacy, "stored_metadata", "unknown")
    readonly property int reportedMaxRetentionDays: integerValue(
        privacy, "max_retention_days", -1)
    readonly property bool memoryCommandAvailable: boolValue(
        privacy, "memory_command_available", false)
    readonly property bool memoryCommandAvailableReported: hasBoolean(
        privacy, "memory_command_available")
    readonly property bool memoryStoreAvailable: boolValue(
        privacy, "memory_store_available", false)
    readonly property bool memoryStoreAvailableReported: hasBoolean(
        privacy, "memory_store_available")
    readonly property bool canMutateSubjects: canMutateSettings
        && memoryStoreAvailableReported && memoryStoreAvailable
    readonly property string memoryIntegrity: stringValue(
        privacy, "memory_integrity", "unknown")
    readonly property bool memoryAvailable: memoryStoreAvailable && memoryCommandAvailable
        && memoryIntegrity === "healthy"
    readonly property bool memoryAvailableReported: memoryCommandAvailableReported
        && memoryStoreAvailableReported && memoryIntegrity !== "unknown"
    readonly property bool learningAvailable: boolValue(privacy, "learning_available", false)
    readonly property bool learningAvailableReported: hasBoolean(privacy, "learning_available")
    readonly property double memoryRecords: integerValue(privacy, "memory_records", -1)
    readonly property double memoryBytes: integerValue(privacy, "memory_bytes", -1)
    readonly property double memoryWriteFailures: integerValue(
        privacy, "memory_write_failures", -1)
    readonly property double memoryDroppedSignals: integerValue(
        privacy, "memory_dropped_signals", -1)

    readonly property string browserPermission: stringValue(support, "browser_permission", "unknown")
    readonly property string badiPolicy: stringValue(support, "badi_policy", "unknown")
    readonly property string supportedScope: stringValue(support, "scope", "")
    readonly property string evidenceClass: stringValue(support, "evidence_class", "unknown")
    readonly property string evidenceCommit: stringValue(support, "evidence_commit", "")

    readonly property string modelStatus: stringValue(writingAdvice, "status", "unknown")
    readonly property bool modelReady: boolValue(writingAdvice, "runtime_ready", false)
    readonly property bool modelReadyReported: hasBoolean(writingAdvice, "runtime_ready")
    readonly property bool modelInstalled: boolValue(writingModel, "installed", false)
    readonly property bool modelInstalledReported: hasBoolean(writingModel, "installed")
    readonly property bool modelConfigured: boolValue(writingModel, "configured", false)
    readonly property bool modelConfiguredReported: hasBoolean(writingModel, "configured")
    readonly property string modelTier: stringValue(writingAdvice, "tier", "unknown")
    readonly property var recommendedModel: objectValue(writingAdvice, "recommended")
    readonly property string modelRepository: stringValue(recommendedModel, "repository", "Not reported")
    readonly property string modelFilename: stringValue(recommendedModel, "filename", "")
    readonly property string modelLicense: stringValue(recommendedModel, "license", "unknown")
    readonly property string modelQuantization: stringValue(recommendedModel, "quantization", "unknown")
    readonly property string modelReason: stringValue(writingAdvice, "reason", "")

    function isObject(value) {
        return value !== null && typeof value === "object" && !Array.isArray(value);
    }

    function objectValue(parent, key) {
        if (!isObject(parent) || !isObject(parent[key])) return ({});
        return parent[key];
    }

    function hasBoolean(parent, key) {
        return isObject(parent) && typeof parent[key] === "boolean";
    }

    function stringValue(parent, key, fallback) {
        if (!isObject(parent) || typeof parent[key] !== "string" || parent[key].length === 0)
            return fallback;
        return parent[key];
    }

    function boolValue(parent, key, fallback) {
        if (!isObject(parent) || typeof parent[key] !== "boolean") return fallback;
        return parent[key];
    }

    function integerValue(parent, key, fallback) {
        if (!isObject(parent) || !Number.isSafeInteger(parent[key])) return fallback;
        return parent[key];
    }

    function hasExactKeys(value, keys) {
        if (!isObject(value)) return false;
        const actual = Object.keys(value).sort();
        const expected = keys.slice().sort();
        if (actual.length !== expected.length) return false;
        for (let index = 0; index < expected.length; index += 1) {
            if (actual[index] !== expected[index]) return false;
        }
        return true;
    }

    function isDecision(value) {
        return value === "allow" || value === "block";
    }

    function isRetention(value) {
        if (!isObject(value) || typeof value.mode !== "string") return false;
        if (value.mode === "none") return hasExactKeys(value, ["mode"]);
        return value.mode === "bounded"
            && hasExactKeys(value, ["mode", "days"])
            && Number.isSafeInteger(value.days)
            && value.days >= 1 && value.days <= 90;
    }

    function isBoundedRetention(value) {
        return isRetention(value) && value.mode === "bounded";
    }

    function isIdentity(value) {
        return hasExactKeys(value, ["kind", "adapter", "scheme", "host", "port"])
            && value.kind === "browser_origin"
            && value.adapter === "chromium"
            && (value.scheme === "http" || value.scheme === "https")
            && typeof value.host === "string" && value.host.length > 0
            && Number.isSafeInteger(value.port) && value.port >= 1 && value.port <= 65535;
    }

    function isPermissions(value) {
        if (!hasExactKeys(value,
                ["suggest", "display", "context_read", "learn", "retention"])) return false;
        if (!isDecision(value.suggest) || !isDecision(value.display)
                || !isDecision(value.context_read) || !isDecision(value.learn)
                || !isRetention(value.retention)) return false;
        if (value.suggest === "allow"
                && (value.display !== "allow" || value.context_read !== "allow")) return false;
        if (value.learn === "allow"
                && (value.suggest !== "allow" || value.display !== "allow"
                    || value.context_read !== "allow")) return false;
        return value.retention.mode !== "bounded" || value.learn === "allow";
    }

    function isSubject(value) {
        return hasExactKeys(value, ["identity", "permissions"])
            && isIdentity(value.identity) && isPermissions(value.permissions);
    }

    function isSettingsDocument(value) {
        if (!hasExactKeys(value, ["schema", "revision", "paused", "subjects"])
                || value.schema !== "badi.settings.v1"
                || !Number.isSafeInteger(value.revision) || value.revision < 0
                || typeof value.paused !== "boolean" || !Array.isArray(value.subjects)
                || value.subjects.length > 64) {
            return false;
        }
        for (let index = 0; index < value.subjects.length; index += 1) {
            if (!isSubject(value.subjects[index])) return false;
            if (index > 0 && compareIdentities(
                    value.subjects[index - 1].identity,
                    value.subjects[index].identity) >= 0) return false;
        }
        return true;
    }

    function isFixtureIdentity(identity) {
        return isIdentity(identity)
            && identity.kind === "browser_origin"
            && identity.adapter === "chromium"
            && identity.scheme === "http"
            && identity.host === "localhost"
            && identity.port === 4173;
    }

    function findFixtureSubject(subjects) {
        if (!Array.isArray(subjects)) return -1;
        for (let index = 0; index < subjects.length; index += 1) {
            if (isSubject(subjects[index]) && isFixtureIdentity(subjects[index].identity))
                return index;
        }
        return -1;
    }

    function fixtureDecision(permission) {
        if (!settingsDocumentValid) return "unknown";
        if (fixtureSubjectIndex < 0) return "block";
        return stringValue(fixturePermissions, permission, "unknown");
    }

    function compareIdentities(left, right) {
        const fields = ["kind", "adapter", "scheme", "host"];
        for (let index = 0; index < fields.length; index += 1) {
            const field = fields[index];
            if (left[field] < right[field]) return -1;
            if (left[field] > right[field]) return 1;
        }
        return left.port - right.port;
    }

    function cloneSettings() {
        if (!settingsDocumentValid) return null;
        return JSON.parse(JSON.stringify(settings));
    }

    function safeError(stderrText, fallback) {
        if (typeof stderrText !== "string") return fallback;
        const oneLine = stderrText.trim().split("\n")[0];
        if (oneLine.length === 0) return fallback;
        return oneLine.slice(0, 180);
    }

    function refresh() {
        if (overviewProcess.running) return;
        if (mutationProcess.running) return;
        errorMessage = "";
        overviewTimedOut = false;
        overviewProcess.exec(["badictl", "overview", "--json"]);
    }

    function runMutation(argv, successMessage) {
        if (mutationProcess.running || overviewProcess.running) {
            noticeTone = "warning";
            noticeMessage = "Badi status or another change is still in progress.";
            return;
        }
        pendingSuccessMessage = successMessage;
        noticeMessage = "";
        mutationTimedOut = false;
        mutationProcess.exec(argv);
    }

    function replaceSettings(document, successMessage) {
        if (!canMutateSettings || document === null) {
            noticeTone = "danger";
            noticeMessage = "The complete settings document is unavailable. Refresh before changing policy.";
            return;
        }
        if (settingsRevision >= Number.MAX_SAFE_INTEGER) {
            noticeTone = "danger";
            noticeMessage = "The settings revision cannot be advanced safely.";
            return;
        }
        document.revision = settingsRevision + 1;
        if (!isSettingsDocument(document)) {
            noticeTone = "danger";
            noticeMessage = "The requested change did not produce a valid badi.settings.v1 document.";
            return;
        }
        runMutation([
            "badictl", "settings", "replace",
            "--if-revision", String(settingsRevision),
            "--json", JSON.stringify(document)
        ], successMessage);
    }

    function setPaused(paused) {
        const document = cloneSettings();
        if (document !== null) document.paused = paused;
        replaceSettings(
            document,
            paused ? "Badi is paused." : "Badi is active."
        );
    }

    function toggleEffectivePause() {
        if (!brokerPaused) {
            if (canMutateSettings) {
                setPaused(true);
            } else if (canRuntimeControl) {
                runMutation(
                    ["badictl", "pause", "on"],
                    "Badi is paused for this broker process; persisted settings were unavailable."
                );
            }
            return;
        }
        if (settingsPaused) {
            setPaused(false);
            return;
        }
        if (canRuntimeControl && !controlPlaneDegraded) {
            runMutation(
                ["badictl", "pause", "off"],
                "Badi's runtime pause was cleared."
            );
            return;
        }
        noticeTone = "danger";
        noticeMessage = "Badi is fail-closed after a control-plane error. Repair or restart it before resuming.";
    }

    function setFixtureSuggestionBundle(allowed) {
        const document = cloneSettings();
        if (document === null) {
            replaceSettings(null, "");
            return;
        }
        let index = findFixtureSubject(document.subjects);
        if (index < 0) {
            if (!allowed) return;
            if (document.subjects.length >= 64) {
                noticeTone = "warning";
                noticeMessage = "The 64-subject settings limit is reached. Remove a subject before adding localhost:4173.";
                return;
            }
            document.subjects.push({
                "identity": {
                    "kind": "browser_origin",
                    "adapter": "chromium",
                    "scheme": "http",
                    "host": "localhost",
                    "port": 4173
                },
                "permissions": {
                    "suggest": "block",
                    "display": "block",
                    "context_read": "block",
                    "learn": "block",
                    "retention": { "mode": "none" }
                }
            });
            document.subjects.sort((left, right) =>
                compareIdentities(left.identity, right.identity));
            index = findFixtureSubject(document.subjects);
        }
        const permissions = document.subjects[index].permissions;
        permissions.context_read = allowed ? "allow" : "block";
        permissions.display = allowed ? "allow" : "block";
        permissions.suggest = allowed ? "allow" : "block";
        if (!allowed) {
            permissions.learn = "block";
            permissions.retention = { "mode": "none" };
        }
        replaceSettings(
            document,
            allowed
                ? "The localhost:4173 suggestion bundle is allowed."
                : "The localhost:4173 suggestion bundle is blocked."
        );
    }

    function setOutcomeAggregates(enabled) {
        const document = cloneSettings();
        if (document === null) {
            replaceSettings(null, "");
            return;
        }
        const index = findFixtureSubject(document.subjects);
        if (index < 0 || !fixtureSuggestionBundleAllowed) {
            noticeTone = "danger";
            noticeMessage = "Allow the localhost:4173 suggestion bundle before retaining outcome aggregates.";
            return;
        }
        const permissions = document.subjects[index].permissions;
        permissions.learn = enabled ? "allow" : "block";
        if (!enabled) permissions.retention = { "mode": "none" };
        replaceSettings(
            document,
            enabled
                ? "Text-free outcome aggregates are on in memory-only mode for localhost:4173."
                : "Outcome aggregates are off for localhost:4173."
        );
    }

    function setRetentionDays(days) {
        if (days !== 0 && days !== 7 && days !== 30 && days !== 90) return;
        const document = cloneSettings();
        if (document === null) {
            replaceSettings(null, "");
            return;
        }
        const index = findFixtureSubject(document.subjects);
        if (index < 0 || !fixtureOutcomeAggregatesAllowed) {
            noticeTone = "danger";
            noticeMessage = "Enable outcome aggregates before changing storage retention.";
            return;
        }
        const permissions = document.subjects[index].permissions;
        if ((days === 0 && permissions.retention.mode === "none")
                || (days > 0 && permissions.retention.mode === "bounded"
                    && permissions.retention.days === days)) return;
        permissions.retention = days === 0
            ? { "mode": "none" }
            : { "mode": "bounded", "days": days };
        replaceSettings(
            document,
            days === 0
                ? "Outcome aggregates are now memory only for localhost:4173."
                : "Outcome aggregate retention is now " + days + " days for localhost:4173."
        );
    }

    function clearMemory() {
        runMutation(
            ["badictl", "memory", "clear"],
            "Local text-free origin/day aggregates were cleared."
        );
    }

    property string pendingSuccessMessage: ""

    Timer {
        id: overviewTimeout
        interval: 5000
        repeat: false
        onTriggered: {
            if (!overviewProcess.running) return;
            root.overviewTimedOut = true;
            overviewProcess.signal(15);
            overviewKillTimeout.restart();
        }
    }

    Timer {
        id: overviewKillTimeout
        interval: 500
        repeat: false
        onTriggered: {
            if (overviewProcess.running) overviewProcess.signal(9);
        }
    }

    Timer {
        id: mutationTimeout
        interval: 5000
        repeat: false
        onTriggered: {
            if (!mutationProcess.running) return;
            root.mutationTimedOut = true;
            mutationProcess.signal(15);
            mutationKillTimeout.restart();
        }
    }

    Timer {
        id: mutationKillTimeout
        interval: 500
        repeat: false
        onTriggered: {
            if (mutationProcess.running) mutationProcess.signal(9);
        }
    }

    Process {
        id: overviewProcess

        stdout: StdioCollector { id: overviewStdout }
        stderr: StdioCollector { id: overviewStderr }

        onStarted: overviewTimeout.restart()
        onExited: (exitCode, exitStatus) => {
            overviewTimeout.stop();
            overviewKillTimeout.stop();
            if (root.overviewTimedOut) {
                root.overview = ({});
                root.errorMessage = "Reading Badi status exceeded the 5 second deadline.";
                return;
            }
            if (exitCode !== 0) {
                root.overview = ({});
                root.errorMessage = root.safeError(
                    overviewStderr.text,
                    "Could not read Badi status. Is badictl installed and the broker available?"
                );
                return;
            }

            try {
                const parsed = JSON.parse(overviewStdout.text);
                if (!root.isObject(parsed) || parsed.schema !== "badi.overview.v1")
                    throw new Error("Unexpected overview schema");
                root.overview = parsed;
                root.errorMessage = "";
                root.lastRefreshed = new Date();
            } catch (error) {
                root.overview = ({});
                root.errorMessage = "badictl returned an invalid badi.overview.v1 document.";
            }
        }
    }

    Process {
        id: mutationProcess

        stdout: StdioCollector {}
        stderr: StdioCollector { id: mutationStderr }

        onStarted: mutationTimeout.restart()
        onExited: (exitCode, exitStatus) => {
            mutationTimeout.stop();
            mutationKillTimeout.stop();
            const succeeded = exitCode === 0 && !root.mutationTimedOut;
            root.noticeTone = succeeded ? "positive" : "danger";
            root.noticeMessage = root.mutationTimedOut
                ? "The Badi change exceeded the 5 second deadline. Status was refreshed."
                : succeeded
                    ? root.pendingSuccessMessage
                    : root.safeError(mutationStderr.text, "Badi rejected the requested change.");
            root.pendingSuccessMessage = "";
            Qt.callLater(() => root.refresh());
        }
    }

    Component.onCompleted: refresh()
}
