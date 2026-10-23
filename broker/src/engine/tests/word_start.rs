//! Letters typed at a word start in front of another word (`the fin|report`)
//! are a new word: the provider is asked as if a space followed them, and
//! the suggestion ends in that space.

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

/// Answers the rest of the typed word, as the local model's mid-line rule
/// does, and records what it was asked.
#[derive(Default)]
struct WordProvider {
    requests: Mutex<Vec<ProviderRequest>>,
}

#[async_trait]
impl CompletionProvider for WordProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.requests.lock().expect("requests").push(request);
        Ok(Some("al".to_owned()))
    }
}

struct Fixture {
    broker: Broker,
    provider: Arc<WordProvider>,
    _storage: TempDir,
}

fn fixture() -> Fixture {
    let site = StableIdentity::browser_origin(
        BrowserAdapter::Chromium,
        WebScheme::Http,
        "localhost",
        Some(4173),
    )
    .expect("identity");
    let storage = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        storage.path().join("config/badi"),
        storage.path().join("data/badi"),
    )
    .expect("paths");
    let control_plane = Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(
            0,
            SettingsV2 {
                revision: 1,
                paused: false,
                mid_line: true,
                subjects: vec![SubjectRule {
                    identity: site,
                    permissions: SubjectPermissions {
                        suggest: PermissionDecision::Allow,
                        display: PermissionDecision::Allow,
                        context_read: PermissionDecision::Allow,
                        learn: PermissionDecision::Block,
                        retention: RetentionPermission::None,
                    },
                }],
                ..SettingsV2::deny_by_default()
            },
        )
        .expect("initial settings");
    let provider = Arc::new(WordProvider::default());
    let broker = Broker::with_control_plane(
        Arc::clone(&provider) as Arc<dyn CompletionProvider>,
        BrokerConfig::default(),
        control_plane,
    )
    .expect("broker");
    Fixture {
        broker,
        provider,
        _storage: storage,
    }
}

struct Field {
    session_id: SessionId,
    events: mpsc::Receiver<BrokerEvent>,
    revision: u64,
}

impl Field {
    async fn open(broker: &Broker) -> Self {
        Self::open_as(broker, "field-word-start").await
    }

    /// Opens a session on the fixture site; observed fields reopen theirs,
    /// with a new target id, on each edit.
    async fn open_as(broker: &Broker, target_id: &str) -> Self {
        let session_id = SessionId::new();
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
                        kind: TargetKind::Browser,
                        app_id: "chromium".to_owned(),
                        target_id: target_id.to_owned(),
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
        Self {
            session_id,
            events,
            revision: 0,
        }
    }

    /// Publishes the field's text around the caret.
    async fn context(&mut self, broker: &Broker, before: &str, after: &str) {
        self.revision += 1;
        let caret = u64::try_from(before.chars().count()).expect("caret");
        broker
            .update_context(
                self.coordinates(),
                ContextChangedPayload {
                    fingerprint: format!("fingerprint_word_start_{:02}", self.revision),
                    before: before.to_owned(),
                    after: after.to_owned(),
                    selection: Selection {
                        anchor: caret,
                        head: caret,
                        unit: OffsetUnit::UnicodeScalarValues,
                    },
                    field: FieldDescriptor {
                        purpose: FieldPurpose::Normal,
                        editable: true,
                        multiline: false,
                        composing: false,
                        sensitive: false,
                        identity_known: true,
                        focused: true,
                        lock_screen: false,
                    },
                    activation: Activation::Always,
                    explicit: true,
                    language: Some("en".to_owned()),
                    window_text: None,
                },
            )
            .await
            .expect("context");
    }

    /// Asks for a suggestion at the current text and returns the shown text,
    /// or `None` when the broker cleared instead.
    async fn suggestion(&mut self, broker: &Broker) -> Option<String> {
        broker
            .request_suggestion(
                self.coordinates(),
                SuggestRequestPayload {
                    fingerprint: format!("fingerprint_word_start_{:02}", self.revision),
                    explicit: true,
                },
                None,
            )
            .await
            .expect("request");
        let event = timeout(Duration::from_secs(2), self.events.recv())
            .await
            .expect("event deadline")
            .expect("event");
        match event {
            BrokerEvent::SuggestionShow { payload, .. } => Some(payload.text),
            BrokerEvent::SuggestionClear { .. } => None,
            other @ BrokerEvent::CommitPrepare { .. } => panic!("unexpected {other:?}"),
        }
    }

    const fn coordinates(&self) -> Coordinates {
        Coordinates {
            session_id: self.session_id,
            focus_epoch: 1,
            revision: self.revision,
        }
    }
}

#[tokio::test]
async fn letters_typed_in_front_of_a_word_are_a_new_word() {
    let fixture = fixture();
    let mut field = Field::open(&fixture.broker).await;
    // The caret is placed in front of `report`, then `fin` is typed there.
    field
        .context(&fixture.broker, "Please find the ", "report today")
        .await;
    field
        .context(&fixture.broker, "Please find the fin", "report today")
        .await;
    assert_eq!(
        field.suggestion(&fixture.broker).await.as_deref(),
        Some("al "),
        "the space keeps the new word apart from `report`"
    );
    let asked = fixture
        .provider
        .requests
        .lock()
        .expect("requests")
        .pop()
        .expect("a provider request");
    assert_eq!(asked.before, "Please find the fin");
    assert_eq!(asked.after, " report today");
}

#[tokio::test]
async fn a_caret_moved_into_a_word_is_not_a_word_start() {
    let fixture = fixture();
    let mut field = Field::open(&fixture.broker).await;
    // No word start was seen in this field: `fin|report` splits a word.
    field
        .context(&fixture.broker, "Please find the fin", "report today")
        .await;
    assert_eq!(field.suggestion(&fixture.broker).await, None);
    // A word start whose text after the caret changed, or that saw a
    // non-letter typed, no longer holds.
    for (typed, after) in [
        ("Please find the fin", "reports today"),
        ("Please find the f-in", "report today"),
    ] {
        let mut field = Field::open(&fixture.broker).await;
        field
            .context(&fixture.broker, "Please find the ", "report today")
            .await;
        field.context(&fixture.broker, typed, after).await;
        assert_eq!(
            field.suggestion(&fixture.broker).await,
            None,
            "{typed:?} | {after:?}"
        );
    }
    assert!(
        fixture
            .provider
            .requests
            .lock()
            .expect("requests")
            .iter()
            .all(|request| !request.after.starts_with(' ')),
        "no request pretended a space there"
    );
}

#[tokio::test]
async fn the_word_start_outlives_a_reopened_session_of_the_same_site() {
    let fixture = fixture();
    let mut field = Field::open_as(&fixture.broker, "visit-1").await;
    field
        .context(&fixture.broker, "Please find the ", "report today")
        .await;
    fixture
        .broker
        .close_session(field.coordinates())
        .await
        .expect("close");
    // The observer reopened the field after the first typed letter.
    let mut reopened = Field::open_as(&fixture.broker, "visit-2").await;
    reopened
        .context(&fixture.broker, "Please find the fin", "report today")
        .await;
    assert_eq!(
        reopened.suggestion(&fixture.broker).await.as_deref(),
        Some("al ")
    );
}
