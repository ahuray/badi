//! Style learning end to end: an accepted suggestion keeps the authored
//! sentence before it only when the user opted in and the subject allows it.

use std::sync::Arc;

use async_trait::async_trait;
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

use crate::control_plane::ControlPlane;
use crate::engine::{Broker, BrokerConfig, BrokerEvent, BrokerEventSink, SessionAuthority};
use crate::protocol::{
    Activation, AdapterKind, Capability, CommitResultPayload, CommitStatus, ContextChangedPayload,
    ControlAction, Coordinates, FieldDescriptor, FieldPurpose, OffsetUnit, Origin, OriginScheme,
    ProviderKind, Selection, SessionControlRequestPayload, SessionId, SessionOpenPayload,
    SuggestRequestPayload, TargetDescriptor, TargetKind,
};
use crate::provider::{
    CompletionProvider, ProviderError, ProviderOutcome, ProviderReply, ProviderRequest,
    RequestTrigger,
};
use crate::settings::{
    BatteryMode, BrowserAdapter, LinuxAdapter, PermissionDecision, RetentionPermission,
    SETTINGS_SCHEMA, SettingsV2, SettingsValidationError, StableIdentity, StoragePaths,
    SubjectPermissions, SubjectRule, WebScheme,
};
use crate::style::{StyleKey, StyleKeyState, StyleStore};

const BEFORE: &str = "Thanks for the quick reply, I will look into it today. Also the ";
const SENTENCE: &str = "Thanks for the quick reply, I will look into it today.";

struct WordProvider;

#[async_trait]
impl CompletionProvider for WordProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn propose_outcome(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
        _allow_replacement: bool,
        _trigger: RequestTrigger,
    ) -> Result<ProviderReply, ProviderError> {
        Ok(ProviderOutcome::from_text(Some("report".to_owned())).into())
    }
}

fn settings(revision: u64, style_learning: bool, learn: bool) -> SettingsV2 {
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: BatteryMode::Quiet,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        places_chosen: false,
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        style_learning,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects: vec![SubjectRule {
            identity: StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Http,
                "localhost",
                Some(4173),
            )
            .expect("identity"),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: if learn {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Block
                },
                retention: if learn {
                    RetentionPermission::Bounded { days: 30 }
                } else {
                    RetentionPermission::None
                },
            },
        }],
    }
}

fn target() -> TargetDescriptor {
    TargetDescriptor {
        kind: TargetKind::Browser,
        app_id: "chromium".to_owned(),
        target_id: "style-field".to_owned(),
        origin: Some(Origin {
            scheme: OriginScheme::Http,
            host: "localhost".to_owned(),
            port: Some(4173),
        }),
    }
}

fn key() -> StyleKey {
    StyleKey::from_hex(&"5a".repeat(16), &"c3".repeat(32)).expect("key")
}

struct Fixture {
    broker: Broker,
    control_plane: Arc<ControlPlane>,
    _storage: TempDir,
}

fn fixture(initial: SettingsV2) -> Fixture {
    let storage = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        storage.path().join("config/badi"),
        storage.path().join("data/badi"),
    )
    .expect("paths");
    let control_plane = Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, initial)
        .expect("initial settings");
    let broker = Broker::with_control_plane(
        Arc::new(WordProvider),
        BrokerConfig::default(),
        Arc::clone(&control_plane),
    )
    .expect("broker");
    Fixture {
        broker,
        control_plane,
        _storage: storage,
    }
}

/// Shows one suggestion after `BEFORE`, accepts it and reports `status`.
async fn accept(broker: &Broker, status: CommitStatus) {
    accept_in(broker, target(), field(), status).await;
}

async fn accept_in(
    broker: &Broker,
    target: TargetDescriptor,
    field: FieldDescriptor,
    status: CommitStatus,
) {
    let session_id = SessionId::new();
    let coordinates = |revision| Coordinates {
        session_id,
        focus_epoch: 1,
        revision,
    };
    let (adapter_kind, commit_capability) = if status == CommitStatus::DispatchedUnverified {
        (AdapterKind::Fcitx, Capability::CommitDispatchedUnverified)
    } else {
        (AdapterKind::Test, Capability::CommitApplied)
    };
    let (sink, mut events) = mpsc::channel(16);
    broker
        .open_session(
            coordinates(0),
            SessionOpenPayload {
                target,
                activation: Activation::Always,
            },
            SessionAuthority {
                adapter_kind,
                capabilities: vec![
                    Capability::Context,
                    Capability::Suggestion,
                    commit_capability,
                    Capability::Policy,
                ],
            },
            BrokerEventSink::new(sink, CancellationToken::new()),
        )
        .await
        .expect("session");
    let fingerprint = "fingerprint_style_0001".to_owned();
    broker
        .update_context(
            coordinates(1),
            ContextChangedPayload {
                fingerprint: fingerprint.clone(),
                before: BEFORE.to_owned(),
                after: String::new(),
                selection: Selection {
                    anchor: u64::try_from(BEFORE.chars().count()).expect("length"),
                    head: u64::try_from(BEFORE.chars().count()).expect("length"),
                    unit: OffsetUnit::UnicodeScalarValues,
                },
                field,
                activation: Activation::Always,
                explicit: true,
                language: Some("en".to_owned()),
                window_text: None,
            },
        )
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(1),
            SuggestRequestPayload {
                fingerprint: fingerprint.clone(),
                explicit: true,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionShow { payload, .. } = next(&mut events).await else {
        panic!("expected a suggestion");
    };
    broker
        .session_control(
            coordinates(1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id.clone()),
                word: None,
            },
            Some("accept".to_owned()),
        )
        .await
        .expect("accept");
    let BrokerEvent::CommitPrepare { payload, .. } = next(&mut events).await else {
        panic!("expected a commit grant");
    };
    broker
        .commit_result(
            coordinates(1),
            CommitResultPayload {
                fingerprint,
                suggestion_id: payload.suggestion_id,
                status,
            },
        )
        .await
        .expect("commit result");
}

fn field() -> FieldDescriptor {
    FieldDescriptor {
        purpose: FieldPurpose::Normal,
        editable: true,
        multiline: true,
        composing: false,
        sensitive: false,
        identity_known: true,
        focused: true,
        lock_screen: false,
    }
}

async fn next(events: &mut mpsc::Receiver<BrokerEvent>) -> BrokerEvent {
    timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("event deadline")
        .expect("event")
}

#[tokio::test]
async fn an_accepted_suggestion_keeps_the_sentence_written_before_it() {
    let fixture = fixture(settings(1, true, true));
    fixture.broker.deliver_style_key(key()).expect("key");
    accept(&fixture.broker, CommitStatus::Applied).await;
    let samples = fixture.broker.export_style().await.expect("export");
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].text, SENTENCE);
    assert_eq!(
        fixture.broker.style_status().expect("status").key,
        StyleKeyState::Ready
    );

    let status = fixture.broker.forget_style().await.expect("forget");
    assert_eq!(status.samples, 0);
    assert_eq!(
        status.key,
        StyleKeyState::Absent,
        "forget drops the key too"
    );
    assert!(fixture.broker.export_style().await.is_err());
}

#[tokio::test]
async fn opting_out_or_a_blocked_subject_keeps_nothing() {
    for (style_learning, learn, status) in [
        (false, true, CommitStatus::Applied),
        (true, false, CommitStatus::Applied),
        (true, true, CommitStatus::Failed),
    ] {
        let fixture = fixture(settings(1, style_learning, learn));
        fixture.broker.deliver_style_key(key()).expect("key");
        accept(&fixture.broker, status).await;
        assert_eq!(
            fixture.broker.export_style().await.expect("export"),
            vec![],
            "{style_learning} {learn} {status:?}"
        );
    }
}

#[tokio::test]
async fn revoking_learning_in_settings_purges_its_samples() {
    let fixture = fixture(settings(1, true, true));
    fixture.broker.deliver_style_key(key()).expect("key");
    accept(&fixture.broker, CommitStatus::Applied).await;
    // The worker seals samples in queue order; an export waits behind it.
    assert_eq!(
        fixture.broker.export_style().await.expect("export").len(),
        1
    );
    fixture
        .control_plane
        .replace_settings(1, settings(2, true, false))
        .expect("revoke learning");
    assert_eq!(fixture.broker.style_status().expect("status").samples, 0);
}

#[tokio::test]
async fn without_a_key_samples_are_dropped_not_kept_in_the_clear() {
    let fixture = fixture(settings(1, true, true));
    accept(&fixture.broker, CommitStatus::Applied).await;
    // Ordering through the worker: the export runs after the dropped sample.
    assert!(fixture.broker.export_style().await.is_err());
    let status = fixture.broker.style_status().expect("status");
    assert_eq!((status.key, status.samples), (StyleKeyState::Absent, 0));
}

#[tokio::test]
async fn native_linux_apps_seal_export_record_personalization_and_purge_on_revocation() {
    let omawrite = StableIdentity::linux_app(LinuxAdapter::Fcitx, "omawrite").expect("omawrite");
    let nvim = StableIdentity::linux_app(LinuxAdapter::Neovim, "nvim").expect("nvim");
    let mut initial = settings(1, true, false);
    initial.subjects = vec![
        SubjectRule {
            identity: omawrite.clone(),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Allow,
                retention: RetentionPermission::Bounded { days: 30 },
            },
        },
        SubjectRule {
            identity: nvim.clone(),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Allow,
                retention: RetentionPermission::Bounded { days: 14 },
            },
        },
    ];
    initial
        .subjects
        .sort_by(|left, right| left.identity.cmp(&right.identity));

    let fixture = fixture(initial.clone());
    fixture.broker.deliver_style_key(key()).expect("key");
    accept_in(
        &fixture.broker,
        TargetDescriptor {
            kind: TargetKind::DesktopApplication,
            app_id: "omawrite".to_owned(),
            target_id: "doc-1".to_owned(),
            origin: None,
        },
        field(),
        CommitStatus::DispatchedUnverified,
    )
    .await;
    accept_in(
        &fixture.broker,
        TargetDescriptor {
            kind: TargetKind::Neovim,
            app_id: "nvim".to_owned(),
            target_id: "buf-1".to_owned(),
            origin: None,
        },
        field(),
        CommitStatus::Applied,
    )
    .await;

    let samples = fixture.broker.export_style().await.expect("export");
    assert_eq!(samples.len(), 2);
    assert!(
        samples
            .iter()
            .any(|sample| sample.identity == omawrite && sample.text == SENTENCE)
    );
    assert!(
        samples
            .iter()
            .any(|sample| sample.identity == nvim && sample.text == SENTENCE)
    );

    let snapshot = fixture
        .broker
        .control_plane_snapshot()
        .await
        .expect("control plane snapshot");
    assert_eq!(snapshot.personalization.records.len(), 2);
    assert_eq!(fixture.broker.outcome_recorder_health().write_failures, 0);

    let mut revoked = initial;
    revoked.revision = 2;
    for rule in &mut revoked.subjects {
        rule.permissions.learn = PermissionDecision::Block;
        rule.permissions.retention = RetentionPermission::None;
    }
    let after = fixture
        .broker
        .replace_settings(1, revoked)
        .await
        .expect("revoke native-app learning");
    assert_eq!(fixture.broker.style_status().expect("status").samples, 0);
    assert_eq!(after.personalization.records, Vec::new());
}

fn terminal_style_targets() -> [(StableIdentity, TargetDescriptor); 4] {
    [
        (
            StableIdentity::linux_app(LinuxAdapter::Shell, "bash").expect("bash"),
            TargetDescriptor {
                kind: TargetKind::Terminal,
                app_id: "bash".to_owned(),
                target_id: "readline".to_owned(),
                origin: None,
            },
        ),
        (
            StableIdentity::linux_app(LinuxAdapter::Fcitx, "com.mitchellh.ghostty")
                .expect("ghostty"),
            TargetDescriptor {
                kind: TargetKind::DesktopApplication,
                app_id: "com.mitchellh.ghostty".to_owned(),
                target_id: "pty-1".to_owned(),
                origin: None,
            },
        ),
        (
            StableIdentity::linux_app(LinuxAdapter::Fcitx, "foot").expect("foot"),
            TargetDescriptor {
                kind: TargetKind::DesktopApplication,
                app_id: "foot".to_owned(),
                target_id: "pty-2".to_owned(),
                origin: None,
            },
        ),
        (
            StableIdentity::linux_app(LinuxAdapter::Fcitx, "org.codeberg.dnkl.foot")
                .expect("foot rdns"),
            TargetDescriptor {
                kind: TargetKind::DesktopApplication,
                app_id: "org.codeberg.dnkl.foot".to_owned(),
                target_id: "pty-3".to_owned(),
                origin: None,
            },
        ),
    ]
}

async fn open_sensitive_test_session(
    broker: &Broker,
    session_id: SessionId,
) -> mpsc::Receiver<BrokerEvent> {
    let (sink, events) = mpsc::channel(16);
    broker
        .open_session(
            Coordinates {
                session_id,
                focus_epoch: 1,
                revision: 0,
            },
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::DesktopApplication,
                    app_id: "omawrite".to_owned(),
                    target_id: "secret".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                adapter_kind: AdapterKind::Test,
                capabilities: vec![
                    Capability::Context,
                    Capability::Suggestion,
                    Capability::CommitApplied,
                    Capability::Policy,
                ],
            },
            BrokerEventSink::new(sink, CancellationToken::new()),
        )
        .await
        .expect("session");
    events
}

fn test_context_payload(fingerprint: &str, field: FieldDescriptor) -> ContextChangedPayload {
    let length = u64::try_from(BEFORE.chars().count()).expect("length");
    ContextChangedPayload {
        fingerprint: fingerprint.to_owned(),
        before: BEFORE.to_owned(),
        after: String::new(),
        selection: Selection {
            anchor: length,
            head: length,
            unit: OffsetUnit::UnicodeScalarValues,
        },
        field,
        activation: Activation::Always,
        explicit: true,
        language: Some("en".to_owned()),
        window_text: None,
    }
}

async fn assert_sensitive_field_never_seals(broker: &Broker) {
    let mut sensitive_field = field();
    sensitive_field.sensitive = true;
    let session_id = SessionId::new();
    let mut events = open_sensitive_test_session(broker, session_id).await;
    let coords = Coordinates {
        session_id,
        focus_epoch: 1,
        revision: 1,
    };
    assert!(
        broker
            .update_context(
                coords,
                test_context_payload("fingerprint_secret_0001", sensitive_field)
            )
            .await
            .is_err(),
        "sensitive context carrying prose fails closed"
    );
    broker
        .update_context(
            coords,
            test_context_payload("fingerprint_secret_0002", field()),
        )
        .await
        .expect("normal context");
    broker
        .request_suggestion(
            coords,
            SuggestRequestPayload {
                fingerprint: "fingerprint_secret_0002".to_owned(),
                explicit: true,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionShow { payload, .. } = next(&mut events).await else {
        panic!("expected a suggestion");
    };
    broker
        .session_control(
            coords,
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: "fingerprint_secret_0002".to_owned(),
                suggestion_id: Some(payload.suggestion_id),
                word: None,
            },
            Some("accept".to_owned()),
        )
        .await
        .expect("accept");
    let BrokerEvent::CommitPrepare { payload, .. } = next(&mut events).await else {
        panic!("expected a commit grant");
    };
    broker
        .inner
        .state
        .lock()
        .await
        .sessions
        .get_mut(&session_id)
        .expect("session")
        .context
        .as_mut()
        .expect("context")
        .field
        .sensitive = true;
    broker
        .commit_result(
            coords,
            CommitResultPayload {
                fingerprint: "fingerprint_secret_0002".to_owned(),
                suggestion_id: payload.suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("commit result");
}

fn terminal_style_settings(
    terminals: &[(StableIdentity, TargetDescriptor)],
) -> (SettingsV2, SettingsV2) {
    let omawrite = StableIdentity::linux_app(LinuxAdapter::Fcitx, "omawrite").expect("omawrite");
    let allow_learn = SubjectPermissions {
        suggest: PermissionDecision::Allow,
        display: PermissionDecision::Allow,
        context_read: PermissionDecision::Allow,
        learn: PermissionDecision::Allow,
        retention: RetentionPermission::Bounded { days: 30 },
    };
    let allow_suggest = SubjectPermissions {
        learn: PermissionDecision::Block,
        retention: RetentionPermission::None,
        ..allow_learn
    };
    let mut valid = settings(1, true, true);
    valid.subjects.push(SubjectRule {
        identity: omawrite,
        permissions: allow_learn,
    });
    for (identity, _) in terminals {
        let mut rejected = valid.clone();
        rejected.subjects.push(SubjectRule {
            identity: identity.clone(),
            permissions: allow_learn,
        });
        rejected
            .subjects
            .sort_by(|left, right| left.identity.cmp(&right.identity));
        assert_eq!(
            rejected.validate(),
            Err(SettingsValidationError::TerminalLearningUnsupported)
        );
        valid.subjects.push(SubjectRule {
            identity: identity.clone(),
            permissions: allow_suggest,
        });
    }
    valid
        .subjects
        .sort_by(|left, right| left.identity.cmp(&right.identity));
    let mut forged = valid.clone();
    for rule in &mut forged.subjects {
        rule.permissions.learn = PermissionDecision::Allow;
        rule.permissions.retention = RetentionPermission::Bounded { days: 30 };
    }
    (valid, forged)
}

#[tokio::test]
async fn terminals_shells_and_sensitive_fields_never_seal_style_samples_even_with_learn_allow() {
    let terminals = terminal_style_targets();
    let (valid, forged) = terminal_style_settings(&terminals);
    let fixture = fixture(valid);
    fixture.broker.deliver_style_key(key()).expect("key");
    fixture.broker.inner.state.lock().await.settings = Some(forged.clone());

    for (_, terminal_target) in &terminals {
        for purpose in [FieldPurpose::Terminal, FieldPurpose::Normal] {
            let mut terminal_field = field();
            terminal_field.purpose = purpose;
            accept_in(
                &fixture.broker,
                terminal_target.clone(),
                terminal_field,
                CommitStatus::DispatchedUnverified,
            )
            .await;
        }
    }

    let mut terminal_purpose = field();
    terminal_purpose.purpose = FieldPurpose::Terminal;
    for non_terminal in [
        TargetDescriptor {
            kind: TargetKind::DesktopApplication,
            app_id: "omawrite".to_owned(),
            target_id: "terminal-pane".to_owned(),
            origin: None,
        },
        target(),
    ] {
        accept_in(
            &fixture.broker,
            non_terminal,
            terminal_purpose,
            CommitStatus::Applied,
        )
        .await;
    }

    assert_sensitive_field_never_seals(&fixture.broker).await;
    assert_eq!(fixture.broker.export_style().await.expect("export"), vec![]);
    assert_eq!(fixture.broker.style_status().expect("status").samples, 0);

    let direct_dir = tempdir().expect("direct style dir");
    let mut direct_store =
        StyleStore::open(direct_dir.path().join("style.json")).expect("style store");
    direct_store.set_key(key());
    for (terminal_identity, _) in &terminals {
        assert!(
            !direct_store
                .record(&forged, 10, terminal_identity, SENTENCE)
                .expect("terminal style record refused")
        );
    }
    assert_eq!(direct_store.status().samples, 0);
}
