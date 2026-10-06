use std::os::unix::fs::PermissionsExt as _;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tempfile::tempdir;
use tokio::sync::{Barrier, mpsc};
use tokio::time::{Duration, sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::{
    Broker, BrokerConfig, BrokerError, BrokerEvent, BrokerEventSink, ContextOutcome,
    ControlPlaneCondition, MAX_PROVIDER_CONCURRENCY, SessionAuthority,
};
use crate::control_plane::ControlPlane;
use crate::metrics::NoSuggestionReason;
use crate::personalization::PersonalizationSignal;
use crate::protocol::{
    Activation, AdapterKind, Capability, CommitResultPayload, CommitStatus, ContextChangedPayload,
    ControlAction, Coordinates, FieldDescriptor, FieldPurpose, OffsetUnit, Origin, OriginScheme,
    PolicyResolutionReason, ProviderKind, ReasonCode, Selection, SessionControlRequestPayload,
    SessionId, SessionOpenPayload, SuggestRequestPayload, TargetDescriptor, TargetKind,
};
use crate::protocol::{ProbeOutcome, ProbeRequestPayload};
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
use crate::settings::{
    BrowserAdapter, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA, SettingsV2,
    StableIdentity, StoragePaths, SubjectPermissions, SubjectRule, WebScheme,
};

struct CountingProvider {
    calls: AtomicU64,
    bytes: AtomicU64,
    delay: Duration,
}

struct PendingProvider {
    calls: AtomicU64,
    token: std::sync::Mutex<Option<CancellationToken>>,
}

struct ReadyThenPendingProvider {
    calls: AtomicU64,
    token: std::sync::Mutex<Option<CancellationToken>>,
}

impl PendingProvider {
    fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            token: std::sync::Mutex::new(None),
        }
    }

    fn cancellation(&self) -> Option<CancellationToken> {
        self.token.lock().expect("provider token lock").clone()
    }
}

impl ReadyThenPendingProvider {
    fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            token: std::sync::Mutex::new(None),
        }
    }

    fn cancellation(&self) -> Option<CancellationToken> {
        self.token.lock().expect("provider token lock").clone()
    }
}

impl CountingProvider {
    fn new(delay: Duration) -> Self {
        Self {
            calls: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            delay,
        }
    }
}

#[async_trait]
impl CompletionProvider for CountingProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(
            u64::try_from(request.byte_len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        sleep(self.delay).await;
        Ok(Some(format!(" revision {}", request.before)))
    }
}

#[async_trait]
impl CompletionProvider for PendingProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.token.lock().expect("provider token lock") = Some(cancellation);
        std::future::pending().await
    }
}

#[async_trait]
impl CompletionProvider for ReadyThenPendingProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            return Ok(Some(format!(" revision {}", request.before)));
        }
        *self.token.lock().expect("provider token lock") = Some(cancellation.clone());
        cancellation.cancelled().await;
        Err(ProviderError::Cancelled)
    }
}

fn coordinates(session_id: SessionId, revision: u64) -> Coordinates {
    Coordinates {
        session_id,
        focus_epoch: 1,
        revision,
    }
}

fn event_sink(sender: mpsc::Sender<BrokerEvent>) -> BrokerEventSink {
    BrokerEventSink::new(sender, CancellationToken::new())
}

fn context(revision: u64, purpose: FieldPurpose) -> ContextChangedPayload {
    ContextChangedPayload {
        fingerprint: format!("fingerprint_{revision:016}"),
        before: revision.to_string(),
        after: String::new(),
        selection: Selection {
            anchor: 1,
            head: 1,
            unit: OffsetUnit::Utf16CodeUnits,
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
        explicit: false,
        language: Some("en".to_owned()),
    }
}

async fn wait_for_provider_token(provider: &PendingProvider) -> CancellationToken {
    timeout(Duration::from_millis(50), async {
        loop {
            if let Some(cancellation) = provider.cancellation() {
                break cancellation;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider received cancellation token")
}

async fn wait_for_second_provider_token(provider: &ReadyThenPendingProvider) -> CancellationToken {
    timeout(Duration::from_millis(50), async {
        loop {
            if let Some(cancellation) = provider.cancellation() {
                break cancellation;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("second provider call received cancellation token")
}

async fn setup(
    provider: std::sync::Arc<dyn CompletionProvider>,
    config: BrokerConfig,
) -> (Broker, SessionId, mpsc::Receiver<BrokerEvent>) {
    setup_with_capabilities(
        provider,
        config,
        vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
        ],
    )
    .await
}

async fn setup_with_capabilities(
    provider: std::sync::Arc<dyn CompletionProvider>,
    config: BrokerConfig,
    capabilities: Vec<Capability>,
) -> (Broker, SessionId, mpsc::Receiver<BrokerEvent>) {
    let broker = Broker::new(provider, config);
    let session_id = SessionId::new();
    let (sink, receiver) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "field-1".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities,
            },
            event_sink(sink),
        )
        .await
        .expect("open session");
    (broker, session_id, receiver)
}

#[tokio::test]
async fn normal_provider_abstention_clears_without_recording_a_failure() {
    let (broker, id, mut events) = setup(
        std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionClear { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("expected clear")
    };
    assert_eq!(payload.reason, ReasonCode::NoSuggestion);
    let metrics = broker.inner.metrics.snapshot();
    assert_eq!(metrics.provider_errors, 0);
    let breakdown = metrics.no_suggestion.expect("breakdown");
    assert_eq!(breakdown.model_abstained, 1);
    assert_eq!(breakdown.total(), 1);
    assert_eq!(breakdown.last, Some(NoSuggestionReason::ModelAbstained));
}

#[tokio::test]
async fn request_side_abstention_is_counted_separately_from_the_model() {
    let (broker, id, mut events) = setup(
        std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.language = Some("ar".to_owned());
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    assert!(matches!(
        timeout(Duration::from_millis(100), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, .. }))
            if payload.reason == ReasonCode::NoSuggestion
    ));
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.request_abstained, 1);
    assert_eq!(breakdown.total(), 1);
}

#[tokio::test]
async fn superseded_provider_work_is_counted_as_stale_once() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, id, _events) = setup(provider, BrokerConfig::default()).await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let cancellation = wait_for_provider_token(&provider_view).await;
    broker
        .update_context(coordinates(id, 2), context(2, FieldPurpose::Normal))
        .await
        .expect("newer context");
    assert!(cancellation.is_cancelled());
    timeout(Duration::from_millis(100), async {
        while broker
            .metrics()
            .snapshot()
            .no_suggestion
            .is_none_or(|breakdown| breakdown.stale == 0)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stale class recorded");
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.stale, 1);
    assert_eq!(breakdown.total(), 1);
}

struct SpellingProvider(&'static str, &'static str);

#[async_trait]
impl CompletionProvider for SpellingProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }
    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }
    async fn propose(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
        _allow_replacement: bool,
    ) -> Result<Option<crate::provider::WritingProposal>, ProviderError> {
        Ok(Some(crate::provider::WritingProposal {
            text: self.1.to_owned(),
            replace_before: Some(self.0.to_owned()),
        }))
    }
}

#[tokio::test]
async fn spelling_replacement_binds_original_suffix_through_commit() {
    for (original, corrected) in [("teh", "the"), ("teh ", "the ")] {
        for action in [ControlAction::AcceptWord, ControlAction::AcceptAll] {
            assert_spelling_replacement_commits(original, corrected, action).await;
        }
    }
}

async fn assert_spelling_replacement_commits(
    original: &'static str,
    corrected: &'static str,
    action: ControlAction,
) {
    let (broker, id, mut events) = setup_with_capabilities(
        std::sync::Arc::new(SpellingProvider(original, corrected)),
        BrokerConfig::default(),
        vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
            Capability::TextReplacement,
        ],
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = format!("This is {original}");
    update.selection.anchor = u64::try_from(update.before.len()).expect("small fixture");
    update.selection.head = update.selection.anchor;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionShow { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("expected spelling preview")
    };
    assert_eq!(payload.replace_before.as_deref(), Some(original));
    assert_eq!(payload.text, corrected);
    assert_eq!(payload.accept_word, corrected);
    broker
        .session_control(
            coordinates(id, 1),
            SessionControlRequestPayload {
                action,
                fingerprint: update.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
            },
            None,
        )
        .await
        .expect("accept");
    let BrokerEvent::CommitPrepare { payload, .. } = events.recv().await.expect("commit") else {
        panic!("expected commit")
    };
    assert_eq!(payload.replace_before.as_deref(), Some(original));
    assert_eq!(payload.text, corrected);
}

#[tokio::test]
async fn provider_cannot_send_replacements_to_an_old_adapter() {
    let (broker, id, mut events) = setup(
        std::sync::Arc::new(SpellingProvider("teh", "the")),
        BrokerConfig::default(),
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "This is teh".to_owned();
    update.selection.anchor = 11;
    update.selection.head = 11;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionClear { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("expected capability rejection")
    };
    assert_eq!(payload.reason, ReasonCode::InvalidCapability);
}

#[tokio::test]
async fn correction_cannot_change_spacing_or_replace_an_unbound_suffix() {
    for (before, after, original, corrected) in [
        ("This is teh ", "", "teh ", "the"),
        ("This is teh", "", "teh", "the "),
        ("This is teh  ", "", "teh  ", "the  "),
        ("This is teh\t", "", "teh\t", "the\t"),
        ("This is teh ", "", "other ", "their "),
        ("Thisisteh ", "", "teh ", "the "),
        ("This is teh ", "later", "teh ", "the "),
    ] {
        let (broker, id, mut events) = setup_with_capabilities(
            std::sync::Arc::new(SpellingProvider(original, corrected)),
            BrokerConfig::default(),
            vec![
                Capability::Context,
                Capability::Suggestion,
                Capability::CommitApplied,
                Capability::TextReplacement,
            ],
        )
        .await;
        let mut update = context(1, FieldPurpose::Normal);
        update.before = before.to_owned();
        update.after = after.to_owned();
        update.selection.anchor = u64::try_from(before.len()).expect("small fixture");
        update.selection.head = update.selection.anchor;
        broker
            .update_context(coordinates(id, 1), update.clone())
            .await
            .expect("context");
        broker
            .request_suggestion(
                coordinates(id, 1),
                SuggestRequestPayload {
                    fingerprint: update.fingerprint,
                    explicit: false,
                },
                None,
            )
            .await
            .expect("request");
        let BrokerEvent::SuggestionClear { payload, .. } =
            timeout(Duration::from_millis(100), events.recv())
                .await
                .expect("deadline")
                .expect("event")
        else {
            panic!("unbound spelling preview for {before:?}")
        };
        assert_eq!(payload.reason, ReasonCode::InvalidOutput, "{before:?}");
        let metrics = broker.metrics().snapshot();
        assert_eq!(metrics.commits_prepared, 0);
        let breakdown = metrics.no_suggestion.expect("breakdown");
        assert_eq!(breakdown.output_rejected, 1, "{before:?}");
        assert_eq!(breakdown.total(), 1, "{before:?}");
    }
}

#[tokio::test]
async fn spelling_after_space_is_revoked_when_typing_continues() {
    let (broker, id, mut events) = setup_with_capabilities(
        std::sync::Arc::new(SpellingProvider("teh ", "the ")),
        BrokerConfig::default(),
        vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
            Capability::TextReplacement,
        ],
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "This is teh ".to_owned();
    update.selection.anchor = 12;
    update.selection.head = 12;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionShow { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("spelling preview")
    };
    let mut next = context(2, FieldPurpose::Normal);
    next.before = "This is teh n".to_owned();
    next.selection.anchor = 13;
    next.selection.head = 13;
    broker
        .update_context(coordinates(id, 2), next)
        .await
        .expect("new context");
    assert!(
        broker
            .session_control(
                coordinates(id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptWord,
                    fingerprint: update.fingerprint,
                    suggestion_id: Some(payload.suggestion_id),
                },
                None,
            )
            .await
            .is_err()
    );
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, BrokerEvent::CommitPrepare { .. }));
    }
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

struct ReplacementAuthorityProbe(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[async_trait]
impl CompletionProvider for ReplacementAuthorityProbe {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }
    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }
    async fn propose(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
        allow_replacement: bool,
    ) -> Result<Option<crate::provider::WritingProposal>, ProviderError> {
        self.0
            .store(allow_replacement, std::sync::atomic::Ordering::SeqCst);
        // Deliberately ignore the false flag to exercise the broker's
        // independent output gate as well as provider configuration.
        Ok(Some(crate::provider::WritingProposal {
            text: "the ".to_owned(),
            replace_before: Some("teh ".to_owned()),
        }))
    }
}

#[tokio::test]
async fn manual_unknown_identity_never_receives_spelling_authority() {
    let allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (broker, id, mut events) = setup_with_capabilities(
        std::sync::Arc::new(ReplacementAuthorityProbe(std::sync::Arc::clone(&allowed))),
        BrokerConfig::default(),
        vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::TextReplacement,
        ],
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "This is teh ".to_owned();
    update.selection.anchor = 12;
    update.selection.head = 12;
    update.field.identity_known = false;
    update.explicit = true;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("explicit context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: true,
            },
            None,
        )
        .await
        .expect("explicit request");
    let BrokerEvent::SuggestionClear { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("replacement must be rejected")
    };
    assert_eq!(payload.reason, ReasonCode::InvalidCapability);
    assert!(!allowed.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

fn controlled_target() -> TargetDescriptor {
    TargetDescriptor {
        kind: TargetKind::Browser,
        app_id: "chromium".to_owned(),
        target_id: "controlled-field".to_owned(),
        origin: Some(Origin {
            scheme: OriginScheme::Http,
            host: "localhost".to_owned(),
            port: Some(4173),
        }),
    }
}

fn controlled_settings(revision: u64, allowed: bool, learn: bool) -> SettingsV2 {
    let decision = if allowed {
        PermissionDecision::Allow
    } else {
        PermissionDecision::Block
    };
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        subjects: vec![SubjectRule {
            identity: StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Http,
                "localhost",
                Some(4173),
            )
            .expect("controlled identity"),
            permissions: SubjectPermissions {
                suggest: decision,
                display: decision,
                context_read: decision,
                learn: if allowed && learn {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Block
                },
                retention: if allowed && learn {
                    RetentionPermission::Bounded { days: 30 }
                } else {
                    RetentionPermission::None
                },
            },
        }],
    }
}

fn controlled_learning_settings(revision: u64, retention: RetentionPermission) -> SettingsV2 {
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        subjects: vec![SubjectRule {
            identity: StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Http,
                "localhost",
                Some(4173),
            )
            .expect("controlled identity"),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Allow,
                retention,
            },
        }],
    }
}

fn controlled_authority() -> SessionAuthority {
    SessionAuthority {
        protocol_version: 1,
        adapter_kind: AdapterKind::Browser,
        capabilities: vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
            Capability::Policy,
        ],
    }
}

async fn prepare_commit_result_case(
    broker: &Broker,
    session_id: SessionId,
    events: &mut mpsc::Receiver<BrokerEvent>,
    action: ControlAction,
) -> (ContextChangedPayload, String) {
    assert!(matches!(
        action,
        ControlAction::AcceptWord | ControlAction::AcceptAll
    ));
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    let suggestion_id = payload.suggestion_id.clone();
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action,
                fingerprint: update.fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id),
            },
            Some("applied-case".to_owned()),
        )
        .await
        .expect("prepare commit");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::CommitPrepare { request_id, .. })
            if request_id.as_deref() == Some("applied-case")
    ));
    (update, suggestion_id)
}

async fn assert_pre_mutation_state_retired(broker: &Broker, session_id: SessionId) {
    let state = broker.inner.state.lock().await;
    let session = state.sessions.get(&session_id).expect("session");
    assert!(session.context.is_none());
    assert!(session.visible.is_none());
    assert!(session.pending.is_none());
    assert!(session.context_lease_cancellation.is_none());
}

#[tokio::test]
async fn one_hundred_supersessions_never_show_or_commit_stale_text() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::from_millis(8)));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_secs(1),
            ..BrokerConfig::default()
        },
    )
    .await;

    for revision in 1..=101 {
        let update = context(revision, FieldPurpose::Normal);
        broker
            .update_context(coordinates(session_id, revision), update.clone())
            .await
            .expect("context update");
        broker
            .request_suggestion(
                coordinates(session_id, revision),
                SuggestRequestPayload {
                    fingerprint: update.fingerprint,
                    explicit: false,
                },
                Some(format!("request-{revision}")),
            )
            .await
            .expect("suggestion request");
        tokio::task::yield_now().await;
    }

    sleep(Duration::from_millis(30)).await;
    let mut shows = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let BrokerEvent::SuggestionShow { coordinates, .. } = event {
            shows.push(coordinates.revision);
        }
    }
    assert_eq!(shows, vec![101]);

    let fingerprint = context(101, FieldPurpose::Normal).fingerprint;
    let active = broker.active_locator().await.expect("one active session");
    broker
        .session_control(
            coordinates(session_id, 101),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint,
                suggestion_id: active.suggestion_id,
            },
            Some("commit-latest".to_owned()),
        )
        .await
        .expect("latest suggestion remains eligible");
    let (commit, commit_id, commit_text) = timeout(Duration::from_millis(50), async {
        loop {
            if let Some(BrokerEvent::CommitPrepare {
                coordinates,
                payload,
                request_id,
            }) = events.recv().await
            {
                break (coordinates, request_id, payload.text);
            }
        }
    })
    .await
    .expect("commit event");
    assert_eq!(commit.revision, 101);
    assert_eq!(commit_id.as_deref(), Some("commit-latest"));
    assert!(commit_text.contains("101"));
    let metrics = broker.metrics().snapshot();
    // A superseded generation may be cancelled before it produces an
    // output or rejected after a result races cancellation. Both are safe
    // outcomes; neither may render or authorize stale text.
    assert!(metrics.cancellations + metrics.stale_results >= 100);
    assert_eq!(metrics.suggestions_shown, 1);
    assert_eq!(metrics.commits_prepared, 1);
}

#[tokio::test]
async fn hard_denied_context_reaches_no_provider_bytes() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, _events) = setup(provider, BrokerConfig::default()).await;
    let mut denied = context(1, FieldPurpose::Password);
    let mut invalid_denied = denied.clone();
    invalid_denied.before = "never-forward-this-secret".to_owned();
    assert!(
        broker
            .update_context(coordinates(session_id, 1), invalid_denied)
            .await
            .is_err()
    );
    denied.before.clear();
    denied.field.sensitive = true;
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 1), denied.clone())
            .await
            .expect("policy outcome"),
        ContextOutcome::Denied
    );
    assert!(matches!(
        broker
            .request_suggestion(
                coordinates(session_id, 1),
                SuggestRequestPayload {
                    fingerprint: denied.fingerprint,
                    explicit: true,
                },
                None,
            )
            .await,
        Err(BrokerError::NoContext)
    ));
    sleep(Duration::from_millis(150)).await;
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider_view.bytes.load(Ordering::Relaxed), 0);
    assert_eq!(broker.metrics().snapshot().provider_input_bytes, 0);
}

#[tokio::test]
async fn restrictive_session_activation_is_applied_before_context_validation() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let provider_view = std::sync::Arc::clone(&provider);
    let broker = Broker::new(provider, BrokerConfig::default());
    let session_id = SessionId::new();
    let (sink, _events) = mpsc::channel(4);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "never-field".to_owned(),
                    origin: None,
                },
                activation: Activation::Never,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(sink),
        )
        .await
        .expect("open never session");

    let mut claimed_always = context(1, FieldPurpose::Normal);
    claimed_always.before = "must-not-cross-boundary".to_owned();
    assert!(matches!(
        broker
            .update_context(coordinates(session_id, 1), claimed_always)
            .await,
        Err(BrokerError::Protocol(
            crate::protocol::ProtocolError::InvalidPayload
        ))
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 0);
    let state = broker.inner.state.lock().await;
    let session = state.sessions.get(&session_id).expect("session");
    assert_eq!(session.coordinates, coordinates(session_id, 0));
    assert!(session.context.is_none());
    assert!(!session.context_seen_at_coordinates);
}

#[tokio::test]
async fn provider_timeout_explicitly_cancels_its_token() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_millis(5),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let cancellation = wait_for_provider_token(&provider_view).await;
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, .. }))
            if payload.reason == ReasonCode::ProviderTimeout
    ));
    assert!(cancellation.is_cancelled());
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.timeout, 1);
    assert_eq!(breakdown.total(), 1);
}

#[tokio::test]
async fn generation_deadline_includes_debounce_and_cancels_late_work() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::from_millis(2),
            provider_timeout: Duration::from_secs(1),
            generation_timeout: Duration::from_millis(8),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let cancellation = wait_for_provider_token(&provider_view).await;
    assert!(matches!(
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("timeout event"),
        Some(BrokerEvent::SuggestionClear { payload, .. })
            if payload.reason == ReasonCode::ProviderTimeout
    ));
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn shutdown_cancels_provider_work_and_removes_sessions() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, _events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let cancellation = wait_for_provider_token(&provider_view).await;

    broker.shutdown().await;

    assert!(cancellation.is_cancelled());
    assert_eq!(broker.session_count().await, 0);
}

#[tokio::test]
async fn provider_admission_is_global_nonblocking_and_released_on_shutdown() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, first_id, _first_events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            provider_concurrency: 1,
            ..BrokerConfig::default()
        },
    )
    .await;
    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(first_id, 1), first.clone())
        .await
        .expect("first context");
    broker
        .request_suggestion(
            coordinates(first_id, 1),
            SuggestRequestPayload {
                fingerprint: first.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("first request");
    let first_cancellation = wait_for_provider_token(&provider_view).await;

    let second_id = SessionId::new();
    let (second_sink, _second_events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(second_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "field-2".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(second_sink),
        )
        .await
        .expect("second session");
    let second = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(second_id, 1), second.clone())
        .await
        .expect("second context");
    assert!(matches!(
        broker
            .request_suggestion(
                coordinates(second_id, 1),
                SuggestRequestPayload {
                    fingerprint: second.fingerprint,
                    explicit: false,
                },
                None,
            )
            .await,
        Err(BrokerError::ProviderBusy)
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 1);

    broker.shutdown().await;
    assert!(first_cancellation.is_cancelled());
    assert!(broker.inner.provider_admissions.is_closed());
    assert!(broker.inner.shutdown.is_cancelled());
    timeout(Duration::from_millis(50), async {
        while broker.inner.provider_admissions.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider permit released after shutdown cancellation");
}

#[test]
fn provider_admission_configuration_is_clamped_to_hard_bounds() {
    for (configured, expected) in [(0, 1), (usize::MAX, MAX_PROVIDER_CONCURRENCY)] {
        let broker = Broker::new(
            std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
            BrokerConfig {
                provider_concurrency: configured,
                ..BrokerConfig::default()
            },
        );
        assert_eq!(
            broker.inner.provider_admissions.available_permits(),
            expected
        );
    }
}

#[tokio::test]
async fn abrupt_silence_expires_context_and_all_derived_authority() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, _events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(2),
            suggestion_ttl: Duration::from_millis(10),
            context_authority_lease: Duration::from_millis(500),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let provider_cancellation = wait_for_provider_token(&provider_view).await;
    assert!(broker.active_locator().await.is_some());

    sleep(Duration::from_millis(600)).await;

    assert!(provider_cancellation.is_cancelled());
    assert!(broker.active_locator().await.is_none());
    assert_eq!(broker.session_count().await, 1);
    let state = broker.inner.state.lock().await;
    let session = state.sessions.get(&session_id).expect("reusable session");
    assert!(session.context.is_none());
    assert!(session.cancellation.is_none());
    assert!(session.visible.is_none());
    assert!(session.pending.is_none());
    assert!(session.context_lease_cancellation.is_none());
}

#[tokio::test]
async fn newer_context_renews_silence_lease_and_session_remains_reusable() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, _events) = setup(
        provider,
        BrokerConfig {
            context_authority_lease: Duration::from_millis(500),
            ..BrokerConfig::default()
        },
    )
    .await;
    broker
        .update_context(coordinates(session_id, 1), context(1, FieldPurpose::Normal))
        .await
        .expect("first context");
    sleep(Duration::from_millis(300)).await;
    let second = context(2, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 2), second.clone())
        .await
        .expect("newer context");
    sleep(Duration::from_millis(300)).await;

    let active = broker
        .active_locator()
        .await
        .expect("renewed active context");
    assert_eq!(active.revision, 2);
    assert_eq!(active.fingerprint, second.fingerprint);

    sleep(Duration::from_millis(250)).await;
    assert!(broker.active_locator().await.is_none());
    let third = context(3, FieldPurpose::Normal);
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 3), third.clone())
            .await
            .expect("session accepts context after expiry"),
        ContextOutcome::Allowed
    );
    assert_eq!(
        broker
            .active_locator()
            .await
            .expect("reactivated session")
            .fingerprint,
        third.fingerprint
    );
}

#[tokio::test]
async fn full_event_queue_fails_connection_closed_without_phantom_suggestion() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
    );
    let session_id = SessionId::new();
    let initial_coordinates = coordinates(session_id, 0);
    let connection_lifetime = CancellationToken::new();
    let (sink, mut events) = mpsc::channel(1);
    sink.try_send(BrokerEvent::SuggestionClear {
        coordinates: initial_coordinates,
        payload: crate::protocol::SuggestionClearPayload {
            fingerprint: "fingerprint_filler".to_owned(),
            suggestion_id: None,
            reason: ReasonCode::Cancelled,
        },
        request_id: None,
    })
    .expect("fill event queue");
    broker
        .open_session(
            initial_coordinates,
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "field-1".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            BrokerEventSink::new(sink, connection_lifetime.clone()),
        )
        .await
        .expect("open session");
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    timeout(Duration::from_millis(50), async {
        loop {
            let state = broker.inner.state.lock().await;
            let finished = state
                .sessions
                .get(&session_id)
                .is_some_and(|session| session.cancellation.is_none());
            drop(state);
            if finished {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("generation finished");

    let state = broker.inner.state.lock().await;
    assert!(state.sessions[&session_id].visible.is_none());
    drop(state);
    assert_eq!(broker.metrics().snapshot().suggestions_shown, 0);
    assert!(connection_lifetime.is_cancelled());
    assert!(matches!(
        events.try_recv(),
        Ok(BrokerEvent::SuggestionClear { payload, .. })
            if payload.fingerprint == "fingerprint_filler"
    ));
}

#[tokio::test]
async fn manual_session_cannot_be_escalated_by_context_claim() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let provider_view = std::sync::Arc::clone(&provider);
    let broker = Broker::new(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(100),
            ..BrokerConfig::default()
        },
    );
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "manual-field".to_owned(),
                    origin: None,
                },
                activation: Activation::Manual,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(sink),
        )
        .await
        .expect("open manual session");

    let ambient = context(1, FieldPurpose::Normal);
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 1), ambient.clone())
            .await
            .expect("ambient policy"),
        ContextOutcome::ManualRequired
    );
    assert!(matches!(
        broker
            .request_suggestion(
                coordinates(session_id, 1),
                SuggestRequestPayload {
                    fingerprint: ambient.fingerprint,
                    explicit: false,
                },
                None,
            )
            .await,
        Err(BrokerError::NoContext)
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 0);

    let mut explicit = context(2, FieldPurpose::Normal);
    explicit.explicit = true;
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 2), explicit.clone())
            .await
            .expect("explicit policy"),
        ContextOutcome::Allowed
    );
    broker
        .request_suggestion(
            coordinates(session_id, 2),
            SuggestRequestPayload {
                fingerprint: explicit.fingerprint,
                explicit: true,
            },
            None,
        )
        .await
        .expect("manual request");
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionShow { .. }))
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn active_locator_fails_closed_when_two_sessions_claim_focus() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, first_id, _first_events) = setup(provider, BrokerConfig::default()).await;
    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(first_id, 1), first)
        .await
        .expect("first context");

    let second_id = SessionId::new();
    let (second_sink, _second_events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(second_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "field-2".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(second_sink),
        )
        .await
        .expect("second session");
    let second = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(second_id, 1), second)
        .await
        .expect("second context");
    assert!(broker.active_locator().await.is_none());

    let mut blurred = context(2, FieldPurpose::Normal);
    blurred.field.focused = false;
    assert_eq!(
        broker
            .update_context(coordinates(second_id, 2), blurred)
            .await
            .expect("blurred context"),
        ContextOutcome::Denied
    );
    assert_eq!(
        broker
            .active_locator()
            .await
            .expect("only first remains")
            .session_id,
        first_id
    );
}

#[tokio::test]
async fn pause_and_dismiss_cancel_eligibility() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::from_millis(5)));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_secs(1),
            ..BrokerConfig::default()
        },
    )
    .await;
    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), first.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: first.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    assert!(broker.set_paused(true).await);
    sleep(Duration::from_millis(20)).await;
    assert!(events.try_recv().is_err());
    assert!(broker.active_locator().await.is_none());

    assert!(!broker.set_paused(false).await);
    let second = context(2, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 2), second.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 2),
            SuggestRequestPayload {
                fingerprint: second.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    broker
        .session_control(
            coordinates(session_id, 2),
            SessionControlRequestPayload {
                action: ControlAction::Dismiss,
                fingerprint: second.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
            },
            None,
        )
        .await
        .expect("dismiss");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, .. })
            if payload.reason == crate::protocol::ReasonCode::Dismissed
    ));
    assert!(
        broker
            .active_locator()
            .await
            .is_some_and(|active| active.suggestion_id.is_none())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_pair_of_pause_toggles_is_linearizable() {
    let broker = Broker::new(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
    );
    let initial = broker.is_paused().await;
    let barrier = std::sync::Arc::new(Barrier::new(3));

    let first_broker = broker.clone();
    let first_barrier = barrier.clone();
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        first_broker.toggle_paused().await
    });
    let second_broker = broker.clone();
    let second_barrier = barrier.clone();
    let second = tokio::spawn(async move {
        second_barrier.wait().await;
        second_broker.toggle_paused().await
    });

    barrier.wait().await;
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first toggle task");
    let second = second.expect("second toggle task");

    assert_ne!(first, second, "each toggle must observe one transition");
    assert_eq!(broker.is_paused().await, initial);
}

#[tokio::test]
async fn paused_and_stale_addressed_accepts_emit_no_commit_prepare() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_secs(1),
            ..BrokerConfig::default()
        },
    )
    .await;
    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), first.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: first.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    let control = SessionControlRequestPayload {
        action: ControlAction::AcceptAll,
        fingerprint: first.fingerprint.clone(),
        suggestion_id: Some(payload.suggestion_id.clone()),
    };

    assert!(broker.set_paused(true).await);
    assert!(matches!(
        broker
            .session_control(coordinates(session_id, 1), control.clone(), None)
            .await,
        Err(BrokerError::Denied(crate::policy::PolicyReason::Paused))
    ));
    while events.try_recv().is_ok() {}
    assert!(events.try_recv().is_err());

    assert!(!broker.set_paused(false).await);
    let second = context(2, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 2), second)
        .await
        .expect("newer context");
    assert!(matches!(
        broker
            .session_control(coordinates(session_id, 1), control, None)
            .await,
        Err(BrokerError::Stale)
    ));
    assert!(events.try_recv().is_err());
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

#[tokio::test]
async fn expired_suggestion_is_cleared_and_ineligible() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(10),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, .. }))
            if payload.reason == crate::protocol::ReasonCode::Expired
    ));
    assert!(matches!(
        broker
            .session_control(
                coordinates(session_id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptAll,
                    fingerprint: update.fingerprint,
                    suggestion_id: Some(payload.suggestion_id),
                },
                None,
            )
            .await,
        Err(BrokerError::NoSuggestion)
    ));
}

#[tokio::test]
async fn ignored_commit_authorization_expires_and_becomes_ineligible() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(100),
            commit_result_lease: Duration::from_millis(10),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: update.fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id.clone()),
            },
            Some("ignored-authorization".to_owned()),
        )
        .await
        .expect("prepare commit");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::CommitPrepare { request_id, .. })
            if request_id.as_deref() == Some("ignored-authorization")
    ));
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, request_id, .. }))
            if payload.reason == crate::protocol::ReasonCode::Expired
                && request_id.as_deref() == Some("ignored-authorization")
    ));
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::NoPendingCommit)
    ));
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_prepared, 1);
    assert_eq!(metrics.commit_failures, 1);
}

#[tokio::test]
async fn applied_all_retires_pre_mutation_context_without_continuation() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
    )
    .await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
            .await;

    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint,
                suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("applied all");

    assert_pre_mutation_state_retired(&broker, session_id).await;
    assert!(events.try_recv().is_err());
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_applied, 1);
    assert_eq!(metrics.commit_failures, 0);
}

#[tokio::test]
async fn applied_word_without_rebind_retires_remainder_and_context() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
    )
    .await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptWord)
            .await;

    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint,
                suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("applied without rebind");

    assert_pre_mutation_state_retired(&broker, session_id).await;
    assert!(events.try_recv().is_err());
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_applied, 1);
    assert_eq!(metrics.commit_failures, 0);
}

#[tokio::test]
async fn terminal_commit_failures_revoke_pre_commit_context_authority() {
    for status in [
        CommitStatus::Stale,
        CommitStatus::Blocked,
        CommitStatus::Failed,
    ] {
        let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
        let (broker, session_id, mut events) = setup(
            provider,
            BrokerConfig {
                debounce: Duration::ZERO,
                ..BrokerConfig::default()
            },
        )
        .await;
        let (update, suggestion_id) =
            prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
                .await;

        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint.clone(),
                    suggestion_id,
                    status,
                },
            )
            .await
            .expect("terminal failure result");

        assert_pre_mutation_state_retired(&broker, session_id).await;
        assert!(broker.active_locator().await.is_none());
        assert!(matches!(
            broker
                .request_suggestion(
                    coordinates(session_id, 1),
                    SuggestRequestPayload {
                        fingerprint: update.fingerprint,
                        explicit: false,
                    },
                    None,
                )
                .await,
            Err(BrokerError::NoContext)
        ));
        assert_eq!(broker.metrics().snapshot().commit_failures, 1);
    }
}

#[tokio::test]
async fn commit_result_rechecks_expired_lease_before_timer_runs() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: update.fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id.clone()),
            },
            Some("delayed-result".to_owned()),
        )
        .await
        .expect("prepare commit");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::CommitPrepare { .. })
    ));

    {
        let mut state = broker.inner.state.lock().await;
        state
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.pending.as_mut())
            .expect("pending commit")
            .expires_at = std::time::Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("monotonic clock has at least one millisecond of history");
    }
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::CommitLeaseExpired)
    ));
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, request_id, .. })
            if payload.reason == crate::protocol::ReasonCode::Expired
                && request_id.as_deref() == Some("delayed-result")
    ));
    assert_eq!(broker.metrics().snapshot().commit_failures, 1);
}

#[tokio::test]
async fn mismatched_commit_result_does_not_consume_valid_pending_lease() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
    )
    .await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
            .await;

    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint.clone(),
                    suggestion_id: "s:different".to_owned(),
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::Stale)
    ));
    assert!(
        broker.inner.state.lock().await.sessions[&session_id]
            .pending
            .is_some()
    );

    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint,
                suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("matching result uses retained lease");
    assert_pre_mutation_state_retired(&broker, session_id).await;
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_applied, 1);
    assert_eq!(metrics.commit_failures, 1);
}

#[tokio::test]
async fn failed_commit_prepare_enqueue_rejects_without_pending_state() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    drop(events);

    assert!(matches!(
        broker
            .session_control(
                coordinates(session_id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptAll,
                    fingerprint: update.fingerprint.clone(),
                    suggestion_id: Some(payload.suggestion_id.clone()),
                },
                Some("closed-sink".to_owned()),
            )
            .await,
        Err(BrokerError::EventSinkClosed)
    ));
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::NoPendingCommit)
    ));
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_prepared, 0);
    assert_eq!(metrics.commit_failures, 1);
}

#[tokio::test]
async fn pause_revokes_a_pending_commit_and_rejects_its_result() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(100),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: update.fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id.clone()),
            },
            Some("accept-before-pause".to_owned()),
        )
        .await
        .expect("accept dispatch");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::CommitPrepare { request_id, .. })
            if request_id.as_deref() == Some("accept-before-pause")
    ));

    assert!(broker.set_paused(true).await);
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, request_id, .. })
            if payload.reason == crate::protocol::ReasonCode::Paused
                && request_id.as_deref() == Some("accept-before-pause")
    ));
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::NoPendingCommit)
    ));
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn durable_corrupt_store_deny_ack_advances_epoch_and_revokes_all_live_authority() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    {
        let control_plane = ControlPlane::open(paths.clone()).expect("control plane");
        control_plane
            .replace_settings(0, controlled_settings(1, true, true))
            .expect("allow controlled origin");
        control_plane
            .record_signal(
                StableIdentity::browser_origin(
                    BrowserAdapter::Chromium,
                    WebScheme::Http,
                    "localhost",
                    Some(4173),
                )
                .expect("controlled identity"),
                crate::personalization::PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("seed personalization");
    }
    let personalization_path = paths.personalization_path();
    std::fs::write(&personalization_path, b"corrupt aggregate evidence\n")
        .expect("corrupt aggregate store");
    let corrupt_bytes = std::fs::read(&personalization_path).expect("corrupt bytes");
    let control_plane =
        std::sync::Arc::new(ControlPlane::open(paths.clone()).expect("degraded control plane"));
    assert!(
        !control_plane
            .snapshot()
            .expect("degraded snapshot")
            .personalization_store_available
    );

    let provider = std::sync::Arc::new(ReadyThenPendingProvider::new());
    let broker = Broker::with_control_plane(
        provider.clone(),
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    broker
        .register_policy_client("connection-a".to_owned())
        .await;
    broker
        .register_policy_client("connection-b".to_owned())
        .await;
    let initial_authority = broker.authority_snapshot().await;
    broker
        .acknowledge_authority("connection-a", initial_authority.authority_epoch)
        .await
        .expect("first connection acknowledgement");
    broker
        .acknowledge_authority("connection-b", initial_authority.authority_epoch)
        .await
        .expect("second connection acknowledgement");
    assert_eq!(
        broker.authority_snapshot().await.pending_acknowledgements,
        0
    );

    let commit_session = SessionId::new();
    let (commit_sink, mut commit_events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(commit_session, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(commit_sink),
        )
        .await
        .expect("commit session");
    let (commit_context, suggestion_id) = prepare_commit_result_case(
        &broker,
        commit_session,
        &mut commit_events,
        ControlAction::AcceptAll,
    )
    .await;

    let provider_session = SessionId::new();
    let (provider_sink, mut provider_events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(provider_session, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(provider_sink),
        )
        .await
        .expect("provider session");
    let provider_context = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(provider_session, 1), provider_context.clone())
        .await
        .expect("provider context");
    broker
        .request_suggestion(
            coordinates(provider_session, 1),
            SuggestRequestPayload {
                fingerprint: provider_context.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("provider request");
    let provider_cancellation = wait_for_second_provider_token(&provider).await;

    let denied = controlled_settings(2, false, false);
    let acknowledged = broker
        .replace_settings(1, denied.clone())
        .await
        .expect("durable deny acknowledgement");
    assert_eq!(acknowledged.settings, denied);
    let persisted: SettingsV2 =
        serde_json::from_slice(&std::fs::read(paths.settings_path()).expect("persisted deny"))
            .expect("valid persisted deny");
    assert_eq!(persisted, denied);
    assert_eq!(
        std::fs::read(&personalization_path).expect("preserved corrupt evidence"),
        corrupt_bytes
    );

    let authority = broker.authority_snapshot().await;
    assert!(authority.authority_epoch > initial_authority.authority_epoch);
    assert_eq!(authority.settings_revision, 2);
    assert_eq!(authority.pending_acknowledgements, 2);
    assert!(!authority.control_plane_degraded);
    assert!(provider_cancellation.is_cancelled());
    assert_eq!(broker.session_count().await, 0);

    assert!(matches!(
        commit_events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, request_id, .. })
            if payload.reason == ReasonCode::PolicyNever
                && request_id.as_deref() == Some("applied-case")
    ));
    assert!(provider_events.recv().await.is_none());
    assert!(matches!(
        broker
            .commit_result(
                coordinates(commit_session, 1),
                CommitResultPayload {
                    fingerprint: commit_context.fingerprint,
                    suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::UnknownSession)
    ));
    assert!(matches!(
        broker
            .update_context(
                coordinates(provider_session, 2),
                context(2, FieldPurpose::Normal),
            )
            .await,
        Err(BrokerError::UnknownSession)
    ));
    let policy = broker.resolve_policy(&controlled_target()).await;
    assert!(!policy.context_allowed);
    assert!(!policy.display_allowed);
    assert!(!policy.suggestions_allowed);
    assert!(!policy.learning_allowed);
    let post_ack_session = SessionId::new();
    let (post_ack_sink, _post_ack_events) = mpsc::channel(1);
    assert!(matches!(
        broker
            .open_session(
                coordinates(post_ack_session, 0),
                SessionOpenPayload {
                    target: controlled_target(),
                    activation: Activation::Always,
                },
                controlled_authority(),
                event_sink(post_ack_sink),
            )
            .await,
        Err(BrokerError::Denied(_))
    ));
}

#[tokio::test]
async fn persisted_policy_is_enforced_and_replacement_revokes_live_sessions() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("allow controlled origin");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let session_id = SessionId::new();
    let (sink, _events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
        .expect("allowed session");
    broker
        .update_context(coordinates(session_id, 1), context(1, FieldPurpose::Normal))
        .await
        .expect("allowed context");

    let snapshot = broker
        .replace_settings(1, controlled_settings(2, false, false))
        .await
        .expect("replace settings");
    assert_eq!(snapshot.settings.revision, 2);
    assert_eq!(broker.session_count().await, 0);
    assert!(matches!(
        broker
            .update_context(coordinates(session_id, 2), context(2, FieldPurpose::Normal),)
            .await,
        Err(BrokerError::UnknownSession)
    ));

    let denied_session = SessionId::new();
    let (sink, _events) = mpsc::channel(8);
    assert!(matches!(
        broker
            .open_session(
                coordinates(denied_session, 0),
                SessionOpenPayload {
                    target: controlled_target(),
                    activation: Activation::Always,
                },
                controlled_authority(),
                event_sink(sink),
            )
            .await,
        Err(BrokerError::Denied(_))
    ));
}

#[tokio::test]
async fn all_linux_apps_reports_its_default_so_adapters_can_tell_it_from_a_rule() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    let mut settings = controlled_settings(1, false, false);
    settings.all_linux_apps = true;
    settings.all_web_origins = true;
    control_plane
        .replace_settings(0, settings)
        .expect("all-app default");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let app = |app_id: &str| TargetDescriptor {
        kind: TargetKind::DesktopApplication,
        app_id: app_id.to_owned(),
        target_id: "ic:1".to_owned(),
        origin: None,
    };
    let unlisted = broker.resolve_policy(&app("libreoffice")).await;
    assert!(unlisted.context_allowed && unlisted.suggestions_allowed && !unlisted.learning_allowed);
    assert_eq!(unlisted.reason, PolicyResolutionReason::MatchedDefault);
    let site = broker
        .resolve_policy(&TargetDescriptor {
            origin: Some(Origin {
                scheme: OriginScheme::Https,
                host: "mail.example".to_owned(),
                port: None,
            }),
            ..controlled_target()
        })
        .await;
    assert_eq!(
        site.reason,
        PolicyResolutionReason::MatchedRule,
        "website defaults keep the protocol v1 reason"
    );
}

#[tokio::test]
async fn all_web_origins_opens_unlisted_sites_but_keeps_exact_and_field_denial() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    // The controlled localhost origin carries an exact block.
    let mut settings = controlled_settings(1, false, false);
    settings.all_web_origins = true;
    control_plane
        .replace_settings(0, settings)
        .expect("all-web default");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let unlisted = TargetDescriptor {
        origin: Some(Origin {
            scheme: OriginScheme::Https,
            host: "mail.example".to_owned(),
            port: None,
        }),
        ..controlled_target()
    };
    let policy = broker.resolve_policy(&unlisted).await;
    assert!(policy.context_allowed && policy.suggestions_allowed);
    assert!(!policy.learning_allowed);
    let blocked = broker.resolve_policy(&controlled_target()).await;
    assert!(!blocked.context_allowed && !blocked.suggestions_allowed);
    let native = broker
        .resolve_policy(&TargetDescriptor {
            kind: TargetKind::DesktopApplication,
            app_id: "brave-browser".to_owned(),
            target_id: "ic:1".to_owned(),
            origin: None,
        })
        .await;
    assert!(!native.context_allowed, "native apps keep exact app rules");

    for (target, allowed) in [(unlisted, true), (controlled_target(), false)] {
        let session_id = SessionId::new();
        let (sink, _events) = mpsc::channel(8);
        let opened = broker
            .open_session(
                coordinates(session_id, 0),
                SessionOpenPayload {
                    target,
                    activation: Activation::Always,
                },
                controlled_authority(),
                event_sink(sink),
            )
            .await;
        assert_eq!(opened.is_ok(), allowed);
        if !allowed {
            continue;
        }
        assert_eq!(
            broker
                .update_context(coordinates(session_id, 1), context(1, FieldPurpose::Normal))
                .await
                .expect("allowed context"),
            ContextOutcome::Allowed
        );
        let mut sensitive = context(2, FieldPurpose::Password);
        sensitive.before.clear();
        sensitive.field.sensitive = true;
        assert_eq!(
            broker
                .update_context(coordinates(session_id, 2), sensitive)
                .await
                .expect("sensitive context"),
            ContextOutcome::Denied
        );
    }
}

#[tokio::test]
async fn broker_records_only_content_free_shown_and_dismissed_aggregates() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, true))
        .expect("allow aggregate recording");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
        .expect("allowed session");
    let mut private_context = context(1, FieldPurpose::Normal);
    private_context.before = "private prose that must never persist".to_owned();
    private_context.selection.anchor =
        u64::try_from(private_context.before.encode_utf16().count()).expect("context length");
    private_context.selection.head = private_context.selection.anchor;
    broker
        .update_context(coordinates(session_id, 1), private_context.clone())
        .await
        .expect("allowed context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: private_context.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("suggestion request");
    let shown = timeout(Duration::from_millis(100), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected suggestion show")
    };
    let suggestion_text = payload.text.clone();
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Dismiss,
                fingerprint: private_context.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
            },
            None,
        )
        .await
        .expect("dismiss suggestion");

    let snapshot = broker
        .control_plane_snapshot()
        .await
        .expect("aggregate snapshot");
    assert_eq!(snapshot.personalization.records.len(), 1);
    let record = &snapshot.personalization.records[0];
    assert_eq!(record.shown, 1);
    assert_eq!(record.dismissed, 1);
    assert_eq!(record.accepted_word, 0);
    assert_eq!(record.accepted_all, 0);
    let persisted =
        std::fs::read_to_string(temporary.path().join("data/badi/personalization.json"))
            .expect("persisted aggregate");
    assert!(!persisted.contains(&private_context.before));
    assert!(!persisted.contains(&suggestion_text));
}

#[tokio::test]
async fn memory_clear_detaches_live_suggestions_from_deleted_aggregates() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, true))
        .expect("allow aggregate recording");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
        control_plane,
    )
    .expect("controlled broker");
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
        .expect("allowed session");
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("allowed context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("suggestion request");
    let shown = timeout(Duration::from_millis(100), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected suggestion show")
    };

    let (changed, cleared) = broker
        .clear_personalization()
        .await
        .expect("clear personalization");
    assert!(changed);
    assert!(cleared.personalization.records.is_empty());
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Dismiss,
                fingerprint: update.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
            },
            None,
        )
        .await
        .expect("dismiss still-visible suggestion");

    let after_dismiss = broker
        .control_plane_snapshot()
        .await
        .expect("flush aggregate queue");
    assert!(after_dismiss.personalization.records.is_empty());
    assert_eq!(broker.outcome_recorder_health().write_failures, 0);
}

#[tokio::test]
async fn runtime_pause_ack_fences_pre_pause_outcome_writes() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, true))
        .expect("allow aggregate recording");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
        .expect("allowed session");
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("allowed context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("suggestion request");
    assert!(matches!(
        timeout(Duration::from_millis(100), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionShow { .. }))
    ));

    assert!(broker.set_paused(true).await);
    let fenced = control_plane.snapshot().expect("direct aggregate snapshot");
    assert_eq!(fenced.personalization.records[0].shown, 1);
    assert_eq!(broker.outcome_recorder_health().write_failures, 0);
}

#[tokio::test]
async fn retention_grant_scrubs_pre_consent_memory_without_post_commit_persistence() {
    let temporary = tempdir().expect("temporary directory");
    let data_dir = temporary.path().join("data/badi");
    let paths =
        StoragePaths::new(temporary.path().join("config/badi"), &data_dir).expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(
            0,
            controlled_learning_settings(1, RetentionPermission::None),
        )
        .expect("allow memory-only aggregate recording");
    control_plane
        .record_signal(
            StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Http,
                "localhost",
                Some(4173),
            )
            .expect("controlled identity"),
            crate::personalization::PersonalizationProvider::PhraseV1,
            PersonalizationSignal::Shown,
        )
        .expect("record memory-only aggregate");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let session_id = SessionId::new();
    let (sink, _events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
        .expect("allowed session");

    std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o500))
        .expect("make aggregate directory read-only");
    let replacement = broker
        .replace_settings(
            1,
            controlled_learning_settings(2, RetentionPermission::Bounded { days: 30 }),
        )
        .await
        .expect("retention grant does not persist pre-consent history");
    std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore aggregate directory");
    assert_eq!(replacement.settings.revision, 2);
    assert!(replacement.personalization.records.is_empty());
    assert_eq!(broker.session_count().await, 0);
    assert!(!broker.is_paused().await);
    let policy = broker.resolve_policy(&controlled_target()).await;
    assert!(!policy.paused);
    assert!(policy.context_allowed);
}

#[tokio::test]
async fn rejected_settings_cas_is_fail_closed_while_restoring_old_authority() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("allow controlled origin");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let mut changes = broker.subscribe_authority_changes();

    let replacement = broker
        .replace_settings(99, controlled_settings(100, false, false))
        .await;
    assert!(matches!(replacement, Err(BrokerError::ControlPlane(_))));

    let gated = changes.recv().await.expect("mutation gate event");
    let restored = changes.recv().await.expect("restored authority event");
    assert!(gated.paused);
    assert!(!restored.paused);
    assert_eq!(restored.settings_revision, 1);
    let authority = broker.authority_snapshot().await;
    assert_eq!(authority.settings_revision, 1);
    assert!(!authority.paused);
    assert_eq!(
        control_plane
            .snapshot()
            .expect("disk snapshot")
            .settings
            .revision,
        1
    );
}

fn broker_with_stalled_recorder(
    capacity: usize,
) -> (
    Broker,
    std::sync::mpsc::Receiver<super::outcomes::OutcomeCommand>,
) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(capacity);
    let recorder = super::outcomes::OutcomeRecorder::stalled(sender);
    let broker = Broker::build(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        None,
        None,
        Some(recorder),
    );
    (broker, receiver)
}

#[tokio::test(start_paused = true)]
async fn stalled_memory_clear_is_bounded_and_never_blocks_suggestion_state() {
    let (broker, commands) = broker_with_stalled_recorder(1);
    let clearing = tokio::spawn({
        let broker = broker.clone();
        async move { broker.clear_personalization().await }
    });
    let queued = loop {
        if let Ok(command) = commands.try_recv() {
            break command;
        }
        tokio::task::yield_now().await;
    };
    assert!(matches!(
        queued,
        super::outcomes::OutcomeCommand::Clear { .. }
    ));
    assert!(
        timeout(Duration::from_millis(10), broker.is_paused())
            .await
            .is_ok(),
        "the disk clear must run without the broker state lock"
    );
    assert!(matches!(
        clearing.await.expect("clear task"),
        Err(BrokerError::ControlPlaneTimeout)
    ));

    let (full, _commands) = broker_with_stalled_recorder(0);
    assert!(matches!(
        timeout(Duration::from_millis(10), full.clear_personalization()).await,
        Ok(Err(BrokerError::ControlPlaneUnavailable))
    ));
}

#[tokio::test]
async fn memory_clear_cannot_downgrade_an_unknown_settings_commit() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        control_plane,
    )
    .expect("controlled broker");

    broker.fail_control_plane_mutation_unknown().await;
    broker.fail_control_plane_mutation_recoverable().await;
    let (_, snapshot) = broker
        .clear_personalization()
        .await
        .expect("memory clear remains independently available");

    assert_eq!(snapshot.settings.revision, 0);
    let health = broker.health_snapshot().await;
    assert!(health.control_plane_degraded);
    assert!(health.paused);
    let state = broker.inner.state.lock().await;
    assert_eq!(
        state.control_plane_condition,
        ControlPlaneCondition::RestartRequired
    );
}

#[tokio::test]
async fn concurrent_settings_replacements_are_serialized_through_authority_install() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("allow controlled origin");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let mut changes = broker.subscribe_authority_changes();

    let first_broker = broker.clone();
    let first = tokio::spawn(async move {
        first_broker
            .replace_settings(1, controlled_settings(2, true, false))
            .await
    });
    assert!(changes.recv().await.expect("first mutation gate").paused);
    let second_broker = broker.clone();
    let second = tokio::spawn(async move {
        second_broker
            .replace_settings(2, controlled_settings(3, false, false))
            .await
    });

    assert_eq!(
        first
            .await
            .expect("first task")
            .expect("first replace")
            .settings
            .revision,
        2
    );
    assert_eq!(
        second
            .await
            .expect("second task")
            .expect("second replace")
            .settings
            .revision,
        3
    );
    let authority = broker.authority_snapshot().await;
    assert_eq!(authority.settings_revision, 3);
    assert_eq!(
        control_plane
            .snapshot()
            .expect("disk snapshot")
            .settings
            .revision,
        3
    );
}

fn probe_request(before: &str, language: &str, allow_replacement: bool) -> ProbeRequestPayload {
    ProbeRequestPayload {
        before: before.to_owned(),
        after: String::new(),
        language: Some(language.to_owned()),
        allow_replacement,
        explicit: false,
    }
}

#[tokio::test]
async fn probe_uses_the_provider_and_display_checks_without_session_or_counters() {
    let broker = Broker::new(
        std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    let suggested = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("probe");
    assert_eq!(suggested.outcome, ProbeOutcome::Suggested);
    assert_eq!(suggested.text.as_deref(), Some(" for your time"));
    assert_eq!(suggested.provider, ProviderKind::PhraseV1);
    assert!(suggested.latency_ms.is_some());
    suggested.validate().expect("valid probe result");
    for (before, language, reason) in [
        ("Thank you", "de", NoSuggestionReason::RequestAbstained),
        ("Unmatched", "en", NoSuggestionReason::ModelAbstained),
    ] {
        let result = broker
            .probe(probe_request(before, language, false))
            .await
            .expect("probe");
        assert_eq!(result.outcome, ProbeOutcome::NoSuggestion);
        assert_eq!(result.reason, Some(reason));
        assert_eq!(result.text, None);
    }
    assert!(matches!(
        broker
            .probe(probe_request(&"a".repeat(513), "en", false))
            .await,
        Err(BrokerError::Protocol(_))
    ));
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.provider_calls, 0);
    assert_eq!(metrics.suggestions_shown, 0);
    assert_eq!(metrics.no_suggestion.expect("breakdown").total(), 0);
    assert_eq!(broker.session_count().await, 0);
    assert!(broker.active_locator().await.is_none());

    broker.set_paused(true).await;
    let paused = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("paused probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!(paused.text, None);
    paused.validate().expect("valid paused result");
}

#[tokio::test]
async fn probe_times_out_like_generation_and_cancels_provider_work() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let broker = Broker::new(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_millis(5),
            ..BrokerConfig::default()
        },
    );
    let result = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("probe");
    assert_eq!(result.outcome, ProbeOutcome::NoSuggestion);
    assert_eq!(result.reason, Some(NoSuggestionReason::Timeout));
    assert!(
        provider_view
            .cancellation()
            .expect("provider token")
            .is_cancelled()
    );
}

#[tokio::test]
async fn probe_replacement_requires_explicit_authority_and_a_bound_suffix() {
    let broker = Broker::new(
        std::sync::Arc::new(SpellingProvider("teh ", "the ")),
        BrokerConfig::default(),
    );
    let denied = broker
        .probe(probe_request("This is teh ", "en", false))
        .await
        .expect("probe");
    assert_eq!(denied.reason, Some(NoSuggestionReason::OutputRejected));
    let allowed = broker
        .probe(probe_request("This is teh ", "en", true))
        .await
        .expect("probe");
    assert_eq!(allowed.outcome, ProbeOutcome::Suggested);
    assert_eq!(allowed.text.as_deref(), Some("the "));
    assert_eq!(allowed.replace_before.as_deref(), Some("teh "));
    allowed.validate().expect("valid replacement result");
    let unbound = broker
        .probe(probe_request("This is ten ", "en", true))
        .await
        .expect("probe");
    assert_eq!(unbound.reason, Some(NoSuggestionReason::OutputRejected));
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

/// Holds each completion until the test releases it.
struct GatedProvider {
    calls: AtomicU64,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl CompletionProvider for GatedProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Some(" for your time".to_owned()))
    }
}

#[tokio::test]
async fn probe_withholds_its_result_when_a_pause_arrives_during_inference() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("initial settings");
    let provider = std::sync::Arc::new(GatedProvider {
        calls: AtomicU64::new(0),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let broker = Broker::with_control_plane(
        std::sync::Arc::clone(&provider) as std::sync::Arc<dyn CompletionProvider>,
        BrokerConfig::default(),
        control_plane,
    )
    .expect("controlled broker");
    let start = || {
        let broker = broker.clone();
        tokio::spawn(async move { broker.probe(probe_request("Thank you", "en", false)).await })
    };

    let pending = start();
    provider.entered.notified().await;
    provider.release.notify_one();
    let shown = pending.await.expect("probe task").expect("probe");
    assert_eq!(shown.outcome, ProbeOutcome::Suggested);
    assert_eq!(shown.text.as_deref(), Some(" for your time"));

    // Runtime pause (`badictl pause`) acknowledged while the provider runs.
    let pending = start();
    provider.entered.notified().await;
    assert!(broker.set_paused(true).await);
    provider.release.notify_one();
    let paused = pending.await.expect("probe task").expect("probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!((paused.text.as_deref(), paused.reason), (None, None));
    paused.validate().expect("valid paused result");
    assert!(!broker.set_paused(false).await);

    // Persisted pause (`badi pause`) committed while the provider runs.
    let pending = start();
    provider.entered.notified().await;
    broker
        .replace_settings(
            1,
            SettingsV2 {
                paused: true,
                ..controlled_settings(2, true, false)
            },
        )
        .await
        .expect("persisted pause");
    provider.release.notify_one();
    let paused = pending.await.expect("probe task").expect("probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!(paused.text, None);
    paused.validate().expect("valid paused result");

    // While persisted pause holds, a probe never reaches the provider.
    assert_eq!(provider.calls.load(Ordering::Relaxed), 3);
    let paused = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 3);
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.provider_calls, 0);
    assert_eq!(metrics.no_suggestion.expect("breakdown").total(), 0);
}

/// Answers after `delay` and records the trigger each call received.
struct TriggerProvider {
    delay: Duration,
    triggers: std::sync::Mutex<Vec<crate::provider::RequestTrigger>>,
}

#[async_trait]
impl CompletionProvider for TriggerProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        unreachable!("the broker requests outcomes with a trigger")
    }

    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
        _allow_replacement: bool,
        trigger: crate::provider::RequestTrigger,
    ) -> Result<crate::provider::ProviderOutcome, ProviderError> {
        self.triggers.lock().expect("trigger log").push(trigger);
        sleep(self.delay).await;
        Ok(crate::provider::ProviderOutcome::Proposal(
            crate::provider::WritingProposal {
                text: format!(" revision {}", request.before),
                replace_before: None,
            },
        ))
    }
}

#[test]
fn explicit_generation_limit_stays_between_the_automatic_limit_and_its_ceiling() {
    let broker = |generation: u64, explicit: u64| {
        let broker = Broker::new(
            std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
            BrokerConfig {
                generation_timeout: Duration::from_millis(generation),
                explicit_generation_timeout: Duration::from_millis(explicit),
                ..BrokerConfig::default()
            },
        );
        let config = broker.inner.config;
        (
            config.generation_timeout,
            config.explicit_generation_timeout,
        )
    };
    let defaults = BrokerConfig::default();
    assert_eq!(defaults.generation_timeout, Duration::from_millis(600));
    assert_eq!(
        defaults.explicit_generation_timeout,
        Duration::from_millis(super::MAX_EXPLICIT_GENERATION_TIMEOUT_MS)
    );
    assert_eq!(
        broker(600, 5_000),
        (Duration::from_millis(600), Duration::from_millis(1_250))
    );
    assert_eq!(
        broker(400, 100),
        (Duration::from_millis(400), Duration::from_millis(400))
    );
    assert_eq!(
        broker(9_000, 9_000),
        (Duration::from_millis(600), Duration::from_millis(1_250))
    );
}

#[tokio::test]
async fn explicit_requests_get_the_longer_generation_limit() {
    for explicit in [false, true] {
        let provider = std::sync::Arc::new(TriggerProvider {
            delay: Duration::from_millis(750),
            triggers: std::sync::Mutex::new(Vec::new()),
        });
        let provider_view = std::sync::Arc::clone(&provider);
        let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
        let update = context(1, FieldPurpose::Normal);
        broker
            .update_context(coordinates(session_id, 1), update.clone())
            .await
            .expect("context");
        broker
            .request_suggestion(
                coordinates(session_id, 1),
                SuggestRequestPayload {
                    fingerprint: update.fingerprint,
                    explicit,
                },
                None,
            )
            .await
            .expect("request");
        let event = timeout(Duration::from_millis(1_500), events.recv())
            .await
            .expect("generation result")
            .expect("event");
        let breakdown = broker.metrics().snapshot().no_suggestion;
        if explicit {
            assert!(
                matches!(event, BrokerEvent::SuggestionShow { ref payload, .. }
                    if payload.text == " revision 1"),
                "{event:?}"
            );
            assert_eq!(broker.metrics().snapshot().suggestions_shown, 1);
        } else {
            assert!(
                matches!(event, BrokerEvent::SuggestionClear { ref payload, .. }
                    if payload.reason == ReasonCode::ProviderTimeout),
                "{event:?}"
            );
            assert_eq!(breakdown.expect("breakdown").timeout, 1);
        }
        assert_eq!(
            *provider_view.triggers.lock().expect("trigger log"),
            [crate::provider::RequestTrigger::from_explicit(explicit)]
        );
    }
}

#[tokio::test]
async fn control_requests_and_explicit_probes_are_explicit() {
    let provider = std::sync::Arc::new(TriggerProvider {
        delay: Duration::from_millis(750),
        triggers: std::sync::Mutex::new(Vec::new()),
    });
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Request,
                fingerprint: update.fingerprint,
                suggestion_id: None,
            },
            None,
        )
        .await
        .expect("control request");
    assert!(matches!(
        timeout(Duration::from_millis(1_500), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionShow { .. }))
    ));
    for explicit in [false, true] {
        let mut request = probe_request("Thank you", "en", false);
        request.explicit = explicit;
        let result = broker.probe(request).await.expect("probe");
        if explicit {
            assert_eq!(result.outcome, ProbeOutcome::Suggested);
            assert!(result.latency_ms.is_some_and(|latency| latency >= 750));
        } else {
            assert_eq!(result.reason, Some(NoSuggestionReason::Timeout));
        }
    }
    assert_eq!(
        *provider_view.triggers.lock().expect("trigger log"),
        [
            crate::provider::RequestTrigger::Explicit,
            crate::provider::RequestTrigger::Automatic,
            crate::provider::RequestTrigger::Explicit,
        ]
    );
}

/// An observed Fcitx field: protocol v2 with a native reading window. Each
/// edit opens a new session with a new target id, like the observer's epochs.
async fn open_observed_session(
    broker: &Broker,
    target_id: &str,
) -> (SessionId, mpsc::Receiver<BrokerEvent>) {
    let session_id = SessionId::new();
    let (sink, receiver) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::DesktopApplication,
                    app_id: "org.telegram.desktop".to_owned(),
                    target_id: target_id.to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 2,
                adapter_kind: AdapterKind::Fcitx,
                capabilities: vec![
                    Capability::Context,
                    Capability::Suggestion,
                    Capability::CommitDispatchedUnverified,
                ],
            },
            event_sink(sink),
        )
        .await
        .expect("open observed session");
    (session_id, receiver)
}

/// Publishes `before` at revision 1 of `session_id`, requests, and returns
/// the next shown suggestion, skipping the clear of an earlier one.
async fn shown_after(
    broker: &Broker,
    session_id: SessionId,
    events: &mut mpsc::Receiver<BrokerEvent>,
    before: &str,
) -> Option<crate::protocol::SuggestionShowPayload> {
    let mut update = context(1, FieldPurpose::Normal);
    update.before = before.to_owned();
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    loop {
        match timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionShow { payload, .. })) => break Some(payload),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break None,
        }
    }
}

#[tokio::test]
async fn typing_through_a_suggestion_carries_its_remainder_into_the_next_field_session() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut first_events) = open_observed_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut first_events, "Hello")
        .await
        .expect("generated suggestion");
    assert_eq!(shown.text, " revision Hello");
    broker
        .close_session(coordinates(first, 1))
        .await
        .expect("close");

    let (second, mut second_events) = open_observed_session(&broker, "field-epoch-2").await;
    let carried = shown_after(&broker, second, &mut second_events, "Hello rev")
        .await
        .expect("carried remainder");
    assert_eq!(carried.text, "ision Hello");
    assert_eq!(carried.accept_word, "ision");
    assert_ne!(carried.suggestion_id, shown.suggestion_id);
    assert!(carried.ttl_ms <= shown.ttl_ms);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);

    // A character that differs from the suggestion asks the model again.
    let (third, mut third_events) = open_observed_session(&broker, "field-epoch-3").await;
    let generated = shown_after(&broker, third, &mut third_events, "Hello rex")
        .await
        .expect("generated suggestion");
    assert_eq!(generated.text, " revision Hello rex");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn escape_whole_acceptance_and_pause_end_type_through() {
    for action in [
        Some(ControlAction::Dismiss),
        Some(ControlAction::AcceptAll),
        None,
    ] {
        let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
        let broker = Broker::new(provider.clone(), BrokerConfig::default());
        let (first, mut events) = open_observed_session(&broker, "field-epoch-1").await;
        let shown = shown_after(&broker, first, &mut events, "Hello")
            .await
            .expect("generated suggestion");
        if let Some(action) = action {
            broker
                .session_control(
                    coordinates(first, 1),
                    SessionControlRequestPayload {
                        action,
                        fingerprint: shown.fingerprint.clone(),
                        suggestion_id: Some(shown.suggestion_id.clone()),
                    },
                    None,
                )
                .await
                .expect("control");
        } else {
            assert!(broker.set_paused(true).await);
            assert!(!broker.set_paused(false).await);
        }
        let (second, mut second_events) = open_observed_session(&broker, "field-epoch-2").await;
        let next = shown_after(&broker, second, &mut second_events, "Hello rev")
            .await
            .expect("generated suggestion");
        assert_eq!(next.text, " revision Hello rev", "{action:?}");
        assert_eq!(provider.calls.load(Ordering::Relaxed), 2, "{action:?}");
    }
}

#[tokio::test]
async fn the_remainder_after_a_word_acceptance_carries() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut events) = open_observed_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut events, "Hello")
        .await
        .expect("generated suggestion");
    broker
        .session_control(
            coordinates(first, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptWord,
                fingerprint: shown.fingerprint.clone(),
                suggestion_id: Some(shown.suggestion_id.clone()),
            },
            Some("word".to_owned()),
        )
        .await
        .expect("accept word");
    let Some(BrokerEvent::CommitPrepare { payload, .. }) = events.recv().await else {
        panic!("expected a commit grant");
    };
    assert_eq!(payload.text, " revision");

    let (second, mut second_events) = open_observed_session(&broker, "field-epoch-2").await;
    let carried = shown_after(&broker, second, &mut second_events, "Hello revision")
        .await
        .expect("carried remainder");
    assert_eq!(carried.text, " Hello");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
}
