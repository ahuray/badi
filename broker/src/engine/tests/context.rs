//! Opt-in context end to end (ADR 0010): what a provider request carries,
//! and which words are learned, with context on, off and off for one site.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

use crate::control_plane::ControlPlane;
use crate::engine::{Broker, BrokerConfig, BrokerEvent, BrokerEventSink, SessionAuthority};
use crate::protocol::{
    Activation, AdapterKind, Capability, ContextChangedPayload, Coordinates, FieldDescriptor,
    FieldPurpose, OffsetUnit, Origin, OriginScheme, ProviderKind, Selection, SessionId,
    SessionOpenPayload, SuggestRequestPayload, TargetDescriptor, TargetKind,
};
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
use crate::settings::{
    BrowserAdapter, PermissionDecision, RetentionPermission, SettingsV2, StableIdentity,
    StoragePaths, SubjectPermissions, SubjectRule, WebScheme,
};
use crate::style::{StyleKey, StyleKeyState};
use crate::vocabulary::Vocabulary;

const WINDOW: &str = "Re: Venue deposit\nMarinella Costa wrote: the deposit is due.";

#[derive(Default)]
struct RecordingProvider {
    requests: Mutex<Vec<ProviderRequest>>,
}

#[async_trait]
impl CompletionProvider for RecordingProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.requests.lock().expect("requests").push(request);
        Ok(Some("there".to_owned()))
    }
}

fn site() -> StableIdentity {
    StableIdentity::browser_origin(
        BrowserAdapter::Chromium,
        WebScheme::Http,
        "localhost",
        Some(4173),
    )
    .expect("identity")
}

fn settings(revision: u64, context_words: Option<bool>, off: bool) -> SettingsV2 {
    SettingsV2 {
        revision,
        paused: false,
        context_words,
        context_words_off: if off { vec![site()] } else { Vec::new() },
        subjects: vec![SubjectRule {
            identity: site(),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Block,
                retention: RetentionPermission::None,
            },
        }],
        ..SettingsV2::deny_by_default()
    }
}

fn key() -> StyleKey {
    StyleKey::from_hex(&"5a".repeat(16), &"c3".repeat(32)).expect("key")
}

struct Fixture {
    broker: Broker,
    provider: Arc<RecordingProvider>,
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
    let provider = Arc::new(RecordingProvider::default());
    let broker = Broker::with_control_plane(
        Arc::clone(&provider) as Arc<dyn CompletionProvider>,
        BrokerConfig::default(),
        Arc::clone(&control_plane),
    )
    .expect("broker");
    broker.deliver_style_key(key()).expect("key");
    Fixture {
        broker,
        provider,
        control_plane,
        _storage: storage,
    }
}

/// Opens one browser field, `field_id`, on the fixture site.
async fn open_field(
    broker: &Broker,
    coordinates: Coordinates,
    field_id: &str,
) -> mpsc::Receiver<BrokerEvent> {
    let (sink, events) = mpsc::channel(16);
    broker
        .open_session(
            coordinates,
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "chromium".to_owned(),
                    target_id: field_id.to_owned(),
                    origin: Some(Origin {
                        scheme: OriginScheme::Http,
                        host: "localhost".to_owned(),
                        port: Some(4173),
                    }),
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 2,
                adapter_kind: AdapterKind::Browser,
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

/// Opens one field, types `before` with `window_text` beside it and waits for
/// the broker's answer. Returns the request the provider saw.
async fn type_in_field(
    fixture: &Fixture,
    before: &str,
    window_text: Option<&str>,
    purpose: FieldPurpose,
    field_id: &str,
) -> ProviderRequest {
    let session_id = SessionId::new();
    let coordinates = |revision| Coordinates {
        session_id,
        focus_epoch: 1,
        revision,
    };
    let mut events = open_field(&fixture.broker, coordinates(0), field_id).await;
    let fingerprint = "fingerprint_context_01".to_owned();
    let length = u64::try_from(before.chars().count()).expect("length");
    fixture
        .broker
        .update_context(
            coordinates(1),
            ContextChangedPayload {
                fingerprint: fingerprint.clone(),
                before: before.to_owned(),
                after: String::new(),
                selection: Selection {
                    anchor: length,
                    head: length,
                    unit: OffsetUnit::UnicodeScalarValues,
                },
                field: FieldDescriptor {
                    purpose,
                    editable: true,
                    multiline: true,
                    composing: false,
                    sensitive: false,
                    identity_known: true,
                    focused: true,
                    lock_screen: false,
                },
                activation: Activation::Always,
                explicit: true,
                language: Some("en".to_owned()),
                window_text: window_text.map(str::to_owned),
            },
        )
        .await
        .expect("context");
    fixture
        .broker
        .request_suggestion(
            coordinates(1),
            SuggestRequestPayload {
                fingerprint,
                explicit: true,
            },
            None,
        )
        .await
        .expect("request");
    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("event deadline")
        .expect("event");
    // The fixture's fixed answer may not fit; only the request matters here.
    assert!(
        matches!(
            event,
            BrokerEvent::SuggestionShow { .. } | BrokerEvent::SuggestionClear { .. }
        ),
        "{event:?}"
    );
    fixture
        .provider
        .requests
        .lock()
        .expect("requests")
        .pop()
        .expect("a provider request")
}

#[tokio::test]
async fn context_on_sends_window_words_and_offers_a_word_from_two_fields() {
    let fixture = fixture(settings(1, Some(true), false));
    let first = type_in_field(
        &fixture,
        "Thanks, I asked Okonkwo about the schedule. Also ",
        Some(WINDOW),
        FieldPurpose::Normal,
        "field-10",
    )
    .await;
    let vocabulary = first.vocabulary.expect("context on");
    assert_eq!(
        vocabulary.window,
        Vocabulary::words(WINDOW),
        "the window's words, never its text"
    );
    assert_eq!(
        vocabulary.learned,
        Vec::<String>::new(),
        "nothing learned yet"
    );

    // The same word in a second field makes it a learned word.
    type_in_field(
        &fixture,
        "Hello, Okonkwo agreed yesterday. ",
        None,
        FieldPurpose::Normal,
        "field-11",
    )
    .await;
    fixture
        .broker
        .flush_outcomes_before_pause_ack()
        .await
        .expect("flush");
    let third = type_in_field(&fixture, "Ask Okon", None, FieldPurpose::Normal, "field-1").await;
    assert_eq!(
        third.vocabulary.expect("context on").learned,
        vec!["Okonkwo"]
    );
    let status = fixture.broker.learned_status().expect("status");
    assert_eq!((status.key, status.subjects), (StyleKeyState::Ready, 1));

    // Forgetting removes the words; the shared key stays.
    let status = fixture.broker.forget_learned().await.expect("forget");
    assert_eq!((status.key, status.words), (StyleKeyState::Ready, Some(0)));
}

#[tokio::test]
async fn context_unchosen_off_or_off_for_the_site_sends_and_learns_nothing() {
    for (chosen, off) in [(None, false), (Some(false), false), (Some(true), true)] {
        let fixture = fixture(settings(1, chosen, off));
        for _ in 0..2 {
            let request = type_in_field(
                &fixture,
                "Thanks, I asked Okonkwo about it. ",
                Some(WINDOW),
                FieldPurpose::Normal,
                "field-12",
            )
            .await;
            assert_eq!(request.vocabulary, None, "{chosen:?} {off}");
        }
        fixture
            .broker
            .flush_outcomes_before_pause_ack()
            .await
            .expect("flush");
        assert_eq!(
            fixture.broker.learned_status().expect("status").words,
            Some(0)
        );
    }
}

#[tokio::test]
async fn a_terminal_line_uses_context_but_is_never_learned() {
    let fixture = fixture(settings(1, Some(true), false));
    for _ in 0..2 {
        let request = type_in_field(
            &fixture,
            "export PASSPHRASE correcthorse ",
            None,
            FieldPurpose::Terminal,
            "field-13",
        )
        .await;
        assert!(request.vocabulary.is_some());
    }
    fixture
        .broker
        .flush_outcomes_before_pause_ack()
        .await
        .expect("flush");
    assert_eq!(
        fixture.broker.learned_status().expect("status").words,
        Some(0)
    );
}

#[tokio::test]
async fn turning_context_off_for_a_site_purges_its_learned_words() {
    let fixture = fixture(settings(1, Some(true), false));
    type_in_field(
        &fixture,
        "Thanks, I asked Okonkwo. ",
        None,
        FieldPurpose::Normal,
        "field-14",
    )
    .await;
    fixture
        .broker
        .flush_outcomes_before_pause_ack()
        .await
        .expect("flush");
    assert_eq!(fixture.broker.learned_status().expect("status").subjects, 1);
    fixture
        .control_plane
        .replace_settings(1, settings(2, Some(true), true))
        .expect("off for the site");
    assert_eq!(fixture.broker.learned_status().expect("status").subjects, 0);
}

#[tokio::test]
async fn one_field_visit_counts_a_word_once_however_often_its_session_reopens() {
    let fixture = fixture(settings(1, Some(true), false));
    // An observed field reopens its session as it changes; the visit stays.
    for _ in 0..3 {
        type_in_field(
            &fixture,
            "Thanks, I asked Okonkwo. ",
            None,
            FieldPurpose::Normal,
            "one-visit",
        )
        .await;
    }
    fixture
        .broker
        .flush_outcomes_before_pause_ack()
        .await
        .expect("flush");
    let request = type_in_field(
        &fixture,
        "Ask Okon",
        None,
        FieldPurpose::Normal,
        "one-visit",
    )
    .await;
    assert_eq!(
        request.vocabulary.expect("context on").learned,
        Vec::<String>::new()
    );
}
