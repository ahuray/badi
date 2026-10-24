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
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
use crate::settings::{
    BatteryMode, BrowserAdapter, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA,
    SettingsV2, StableIdentity, StoragePaths, SubjectPermissions, SubjectRule, WebScheme,
};
use crate::style::{StyleKey, StyleKeyState};

const BEFORE: &str = "Thanks for the quick reply, I will look into it today. Also the ";
const SENTENCE: &str = "Thanks for the quick reply, I will look into it today.";

struct WordProvider;

#[async_trait]
impl CompletionProvider for WordProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        Ok(Some("report".to_owned()))
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
    let session_id = SessionId::new();
    let coordinates = |revision| Coordinates {
        session_id,
        focus_epoch: 1,
        revision,
    };
    let (sink, mut events) = mpsc::channel(16);
    broker
        .open_session(
            coordinates(0),
            SessionOpenPayload {
                target: target(),
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
                field: field(),
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
