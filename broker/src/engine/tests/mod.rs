//! Broker engine tests: shared providers, fixtures and helpers; each topic
//! has its own module.

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
use crate::personalization::{DailyAggregate, PersonalizationSignal};
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

mod alternatives;
mod commit;
mod context;
mod desktop;
mod editor;
mod generation;
mod outcomes;
mod policy;
mod probe;
mod quietness;
mod rejections;
mod revision;
mod snooze;
mod spelling;
mod stats;
mod style;
mod type_through;
mod word_start;

struct CountingProvider {
    calls: AtomicU64,
    bytes: AtomicU64,
    delay: Duration,
    languages: std::sync::Mutex<Vec<Option<String>>>,
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
            languages: std::sync::Mutex::new(Vec::new()),
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
        self.languages
            .lock()
            .expect("languages")
            .push(request.language.clone());
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
        explicit: false,
        language: Some("en".to_owned()),
        window_text: None,
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
                adapter_kind: AdapterKind::Test,
                capabilities,
            },
            event_sink(sink),
        )
        .await
        .expect("open session");
    (broker, session_id, receiver)
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
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
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
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
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
        adapter_kind: AdapterKind::Test,
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

fn probe_request(before: &str, language: &str, allow_replacement: bool) -> ProbeRequestPayload {
    ProbeRequestPayload {
        before: before.to_owned(),
        after: String::new(),
        language: Some(language.to_owned()),
        allow_replacement,
        explicit: false,
    }
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
