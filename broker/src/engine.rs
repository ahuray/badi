use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{Mutex, mpsc};
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::metrics::{Metrics, MetricsSnapshot};
use crate::policy::{PolicyDecision, PolicyInput, PolicyReason, evaluate};
use crate::protocol::{
    Acceptance, Activation, ActiveLocator, AdapterKind, Capability, CommitPreparePayload,
    CommitResultPayload, CommitStatus, ContextChangedPayload, Coordinates,
    DEFAULT_SUGGESTION_TTL_MS, MAX_FRAME_BYTES, MAX_SAFE_COUNTER, MessageType, ProviderKind,
    ReasonCode, SessionControlRequestPayload, SessionId, SessionOpenPayload, SuggestCancelPayload,
    SuggestRequestPayload, SuggestionClearPayload, SuggestionShowPayload, WireEnvelope,
};
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
use crate::segment::{OutputError, accept_word, sanitize_suggestion};

/// Default broker-local time allowed for an adapter to report a commit result.
pub const DEFAULT_COMMIT_RESULT_LEASE_MS: u64 = 1_500;
/// Hard ceiling for the broker-local commit-result lease.
pub const MAX_COMMIT_RESULT_LEASE_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug)]
pub struct BrokerConfig {
    pub debounce: Duration,
    pub provider_timeout: Duration,
    pub suggestion_ttl: Duration,
    /// Receiver-local lease; adapters must report the authorized commit before it expires.
    pub commit_result_lease: Duration,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(120),
            provider_timeout: Duration::from_millis(1_300),
            suggestion_ttl: Duration::from_millis(DEFAULT_SUGGESTION_TTL_MS),
            commit_result_lease: Duration::from_millis(DEFAULT_COMMIT_RESULT_LEASE_MS),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionAuthority {
    pub adapter_kind: AdapterKind,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BrokerEvent {
    SuggestionShow {
        coordinates: Coordinates,
        payload: SuggestionShowPayload,
        request_id: Option<String>,
    },
    SuggestionClear {
        coordinates: Coordinates,
        payload: SuggestionClearPayload,
        request_id: Option<String>,
    },
    CommitPrepare {
        coordinates: Coordinates,
        payload: CommitPreparePayload,
        request_id: Option<String>,
    },
}

impl BrokerEvent {
    pub fn into_wire(self, mono_ms: u64) -> Result<WireEnvelope, crate::protocol::ProtocolError> {
        let (mut envelope, request_id) = match self {
            Self::SuggestionShow {
                coordinates,
                payload,
                request_id,
            } => (
                WireEnvelope::session(MessageType::SuggestionShow, coordinates, mono_ms, &payload)?,
                request_id,
            ),
            Self::SuggestionClear {
                coordinates,
                payload,
                request_id,
            } => (
                WireEnvelope::session(
                    MessageType::SuggestionClear,
                    coordinates,
                    mono_ms,
                    &payload,
                )?,
                request_id,
            ),
            Self::CommitPrepare {
                coordinates,
                payload,
                request_id,
            } => (
                WireEnvelope::session(MessageType::CommitPrepare, coordinates, mono_ms, &payload)?,
                request_id,
            ),
        };
        envelope.id = request_id;
        Ok(envelope)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextOutcome {
    Allowed,
    ManualRequired,
    Denied,
}

#[derive(Clone)]
pub struct Broker {
    inner: Arc<BrokerInner>,
}

struct BrokerInner {
    provider: Arc<dyn CompletionProvider>,
    provider_kind: ProviderKind,
    config: BrokerConfig,
    metrics: Arc<Metrics>,
    started: Instant,
    state: Mutex<BrokerState>,
}

#[derive(Default)]
struct BrokerState {
    paused: bool,
    sessions: HashMap<SessionId, SessionState>,
}

struct SessionState {
    coordinates: Coordinates,
    target: SessionOpenPayload,
    authority: SessionAuthority,
    context: Option<StoredContext>,
    context_seen_at_coordinates: bool,
    generation: u64,
    cancellation: Option<CancellationToken>,
    visible: Option<VisibleSuggestion>,
    pending: Option<PendingCommit>,
    sink: mpsc::UnboundedSender<BrokerEvent>,
}

#[derive(Clone)]
struct StoredContext {
    payload: ContextChangedPayload,
}

struct VisibleSuggestion {
    payload: SuggestionShowPayload,
    expires_at: Instant,
    request_id: Option<String>,
}

struct PendingCommit {
    coordinates: Coordinates,
    fingerprint: String,
    suggestion_id: String,
    acceptance: Acceptance,
    remainder: String,
    request_id: Option<String>,
    expires_at: Instant,
}

struct PartialContinuation {
    coordinates: Coordinates,
    fingerprint: String,
}

impl Broker {
    #[must_use]
    pub fn new(provider: Arc<dyn CompletionProvider>, config: BrokerConfig) -> Self {
        let provider_kind = provider.kind();
        let config = BrokerConfig {
            suggestion_ttl: config
                .suggestion_ttl
                .clamp(Duration::from_millis(1), Duration::from_millis(600)),
            commit_result_lease: config.commit_result_lease.clamp(
                Duration::from_millis(1),
                Duration::from_millis(MAX_COMMIT_RESULT_LEASE_MS),
            ),
            ..config
        };
        Self {
            inner: Arc::new(BrokerInner {
                provider,
                provider_kind,
                config,
                metrics: Arc::new(Metrics::default()),
                started: Instant::now(),
                state: Mutex::new(BrokerState::default()),
            }),
        }
    }

    #[must_use]
    pub fn provider_kind(&self) -> ProviderKind {
        self.inner.provider_kind
    }

    #[must_use]
    pub fn metrics(&self) -> Arc<Metrics> {
        Arc::clone(&self.inner.metrics)
    }

    #[must_use]
    pub fn mono_ms(&self) -> u64 {
        u64::try_from(self.inner.started.elapsed().as_millis())
            .unwrap_or(MAX_SAFE_COUNTER)
            .min(MAX_SAFE_COUNTER)
    }

    pub async fn open_session(
        &self,
        coordinates: Coordinates,
        payload: SessionOpenPayload,
        authority: SessionAuthority,
        sink: mpsc::UnboundedSender<BrokerEvent>,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_coordinate_bounds(coordinates.focus_epoch, coordinates.revision)?;
        payload.target.validate()?;
        if !authority.capabilities.contains(&Capability::Context)
            || !authority.capabilities.contains(&Capability::Suggestion)
        {
            return Err(BrokerError::InvalidCapability);
        }
        let mut state = self.inner.state.lock().await;
        if state.sessions.contains_key(&coordinates.session_id) {
            return Err(BrokerError::SessionAlreadyOpen);
        }
        state.sessions.insert(
            coordinates.session_id,
            SessionState {
                coordinates,
                target: payload,
                authority,
                context: None,
                context_seen_at_coordinates: false,
                generation: 0,
                cancellation: None,
                visible: None,
                pending: None,
                sink,
            },
        );
        Ok(())
    }

    pub async fn close_session(&self, coordinates: Coordinates) -> Result<(), BrokerError> {
        let mut state = self.inner.state.lock().await;
        let session = state
            .sessions
            .get(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        ensure_coordinates(session.coordinates, coordinates)?;
        let mut session = state
            .sessions
            .remove(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        retire_session(&mut session, &self.inner.metrics, ReasonCode::SessionClosed);
        Ok(())
    }

    pub async fn close_owned_sessions(&self, session_ids: &[SessionId]) {
        let mut state = self.inner.state.lock().await;
        for session_id in session_ids {
            if let Some(mut session) = state.sessions.remove(session_id) {
                retire_session(&mut session, &self.inner.metrics, ReasonCode::SessionClosed);
            }
        }
    }

    pub async fn update_context(
        &self,
        coordinates: Coordinates,
        mut payload: ContextChangedPayload,
    ) -> Result<ContextOutcome, BrokerError> {
        crate::protocol::validate_coordinate_bounds(coordinates.focus_epoch, coordinates.revision)?;
        payload.validate()?;
        self.inner.metrics.record_context_update();

        let mut state = self.inner.state.lock().await;
        let paused = state.paused;
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        ensure_newer_context(session, coordinates)?;
        retire_session(session, &self.inner.metrics, ReasonCode::Superseded);
        session.coordinates = coordinates;
        session.context_seen_at_coordinates = true;
        payload.activation = restrictive_activation(session.target.activation, payload.activation);

        let decision = evaluate(PolicyInput {
            activation: payload.activation,
            explicit: payload.explicit,
            field: payload.field,
            target_kind: session.target.target.kind,
            paused,
            selection_collapsed: payload.selection.anchor == payload.selection.head,
        });
        match decision {
            PolicyDecision::Allow(_) => {
                session.context = Some(StoredContext { payload });
                Ok(ContextOutcome::Allowed)
            }
            PolicyDecision::ManualRequired(_) => {
                session.context = None;
                self.inner.metrics.record_manual_required();
                Ok(ContextOutcome::ManualRequired)
            }
            PolicyDecision::Deny(_) => {
                session.context = None;
                self.inner.metrics.record_denied();
                Ok(ContextOutcome::Denied)
            }
        }
    }

    pub async fn request_suggestion(
        &self,
        coordinates: Coordinates,
        payload: SuggestRequestPayload,
        request_id: Option<String>,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_fingerprint(&payload.fingerprint)?;
        if request_id
            .as_deref()
            .is_some_and(|request_id| !crate::protocol::valid_opaque_id(request_id))
        {
            return Err(BrokerError::InvalidPayload);
        }

        let (provider_request, cancellation, generation) = {
            let mut state = self.inner.state.lock().await;
            let paused = state.paused;
            let session = state
                .sessions
                .get_mut(&coordinates.session_id)
                .ok_or(BrokerError::UnknownSession)?;
            ensure_coordinates(session.coordinates, coordinates)?;
            let context = session.context.clone().ok_or(BrokerError::NoContext)?;
            if context.payload.fingerprint != payload.fingerprint {
                return Err(BrokerError::Stale);
            }
            match evaluate(PolicyInput {
                activation: context.payload.activation,
                explicit: payload.explicit,
                field: context.payload.field,
                target_kind: session.target.target.kind,
                paused,
                selection_collapsed: context.payload.selection.anchor
                    == context.payload.selection.head,
            }) {
                PolicyDecision::Allow(_) => {}
                PolicyDecision::ManualRequired(_) => {
                    self.inner.metrics.record_manual_required();
                    return Err(BrokerError::ManualRequired);
                }
                PolicyDecision::Deny(reason) => {
                    self.inner.metrics.record_denied();
                    return Err(BrokerError::Denied(reason));
                }
            }

            retire_session(session, &self.inner.metrics, ReasonCode::Superseded);
            session.generation = session.generation.wrapping_add(1);
            let generation = session.generation;
            let cancellation = CancellationToken::new();
            session.cancellation = Some(cancellation.clone());
            (
                ProviderRequest {
                    before: context.payload.before,
                    after: context.payload.after,
                    language: context.payload.language,
                },
                cancellation,
                generation,
            )
        };

        let broker = self.clone();
        tokio::spawn(async move {
            if broker.inner.config.debounce != Duration::ZERO {
                tokio::select! {
                    () = time::sleep(broker.inner.config.debounce) => {}
                    () = cancellation.cancelled() => return,
                }
            }
            if cancellation.is_cancelled() {
                return;
            }
            broker
                .inner
                .metrics
                .record_provider_call(provider_request.byte_len());
            let result = time::timeout(
                broker.inner.config.provider_timeout,
                broker
                    .inner
                    .provider
                    .complete(provider_request, cancellation.clone()),
            )
            .await;
            broker
                .finish_generation(
                    coordinates,
                    payload.fingerprint,
                    generation,
                    request_id,
                    cancellation,
                    result,
                )
                .await;
        });
        Ok(())
    }

    async fn finish_generation(
        &self,
        coordinates: Coordinates,
        fingerprint: String,
        generation: u64,
        request_id: Option<String>,
        cancellation: CancellationToken,
        result: Result<Result<Option<String>, ProviderError>, time::error::Elapsed>,
    ) {
        let output = match result {
            Ok(Ok(Some(raw))) => {
                self.inner.metrics.record_provider_output(raw.len());
                sanitize_suggestion(&raw)
            }
            Ok(Ok(None)) => Err(OutputError::Empty),
            Ok(Err(ProviderError::Cancelled)) => return,
            Ok(Err(ProviderError::Unavailable)) | Err(_) => {
                self.inner.metrics.record_provider_error();
                let reason = if result.is_err() {
                    ReasonCode::ProviderTimeout
                } else {
                    ReasonCode::ProviderError
                };
                self.clear_failed_generation(
                    coordinates,
                    &fingerprint,
                    generation,
                    request_id,
                    reason,
                )
                .await;
                return;
            }
        };

        let Ok(text) = output else {
            self.inner.metrics.record_provider_error();
            self.clear_failed_generation(
                coordinates,
                &fingerprint,
                generation,
                request_id,
                ReasonCode::InvalidOutput,
            )
            .await;
            return;
        };

        let mut state = self.inner.state.lock().await;
        if state.paused {
            self.inner.metrics.record_stale_result();
            return;
        }
        let Some(session) = state.sessions.get_mut(&coordinates.session_id) else {
            self.inner.metrics.record_stale_result();
            return;
        };
        let is_current = session.generation == generation
            && session.coordinates == coordinates
            && session
                .context
                .as_ref()
                .is_some_and(|context| context.payload.fingerprint == fingerprint)
            && !cancellation.is_cancelled();
        if !is_current {
            self.inner.metrics.record_stale_result();
            return;
        }

        session.cancellation = None;
        let suggestion_id = format!("s:{}", uuid::Uuid::new_v4());
        let expires_at = Instant::now() + self.inner.config.suggestion_ttl;
        let payload = SuggestionShowPayload {
            fingerprint,
            suggestion_id: suggestion_id.clone(),
            accept_word: accept_word(&text).accepted,
            text,
            ttl_ms: duration_millis(self.inner.config.suggestion_ttl),
            provider: self.inner.provider_kind,
        };
        session.visible = Some(VisibleSuggestion {
            payload: payload.clone(),
            expires_at,
            request_id: request_id.clone(),
        });
        let _ = session.sink.send(BrokerEvent::SuggestionShow {
            coordinates,
            payload,
            request_id,
        });
        self.inner.metrics.record_suggestion_shown();
        drop(state);

        let broker = self.clone();
        tokio::spawn(async move {
            time::sleep(broker.inner.config.suggestion_ttl).await;
            broker
                .expire_suggestion(coordinates, generation, suggestion_id)
                .await;
        });
    }

    async fn clear_failed_generation(
        &self,
        coordinates: Coordinates,
        fingerprint: &str,
        generation: u64,
        request_id: Option<String>,
        reason: ReasonCode,
    ) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&coordinates.session_id) else {
            return;
        };
        if session.generation != generation || session.coordinates != coordinates {
            self.inner.metrics.record_stale_result();
            return;
        }
        session.cancellation = None;
        let _ = session.sink.send(BrokerEvent::SuggestionClear {
            coordinates,
            payload: SuggestionClearPayload {
                fingerprint: fingerprint.to_owned(),
                suggestion_id: None,
                reason,
            },
            request_id,
        });
    }

    async fn expire_suggestion(
        &self,
        coordinates: Coordinates,
        generation: u64,
        suggestion_id: String,
    ) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&coordinates.session_id) else {
            return;
        };
        let Some(visible) = session.visible.as_ref() else {
            return;
        };
        if session.generation != generation
            || session.coordinates != coordinates
            || visible.payload.suggestion_id != suggestion_id
            || visible.expires_at > Instant::now()
        {
            return;
        }
        let visible = session.visible.take().expect("visible suggestion checked");
        let _ = session.sink.send(BrokerEvent::SuggestionClear {
            coordinates,
            payload: SuggestionClearPayload {
                fingerprint: visible.payload.fingerprint,
                suggestion_id: Some(visible.payload.suggestion_id),
                reason: ReasonCode::Expired,
            },
            request_id: visible.request_id,
        });
        self.inner.metrics.record_suggestion_expired();
    }

    async fn expire_pending_commit(
        &self,
        coordinates: Coordinates,
        suggestion_id: String,
        expires_at: Instant,
    ) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&coordinates.session_id) else {
            return;
        };
        let Some(pending) = session.pending.as_ref() else {
            return;
        };
        if pending.coordinates != coordinates
            || pending.suggestion_id != suggestion_id
            || pending.expires_at != expires_at
            || pending.expires_at > Instant::now()
        {
            return;
        }
        let pending = session.pending.take().expect("pending commit checked");
        send_pending_clear(session, pending, ReasonCode::Expired);
        self.inner.metrics.record_commit_failure();
    }

    pub async fn cancel_suggestion(
        &self,
        coordinates: Coordinates,
        payload: SuggestCancelPayload,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_fingerprint(&payload.fingerprint)?;
        let mut state = self.inner.state.lock().await;
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        ensure_coordinates(session.coordinates, coordinates)?;
        ensure_fingerprint(session, &payload.fingerprint)?;
        retire_session(session, &self.inner.metrics, payload.reason);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub async fn session_control(
        &self,
        coordinates: Coordinates,
        payload: SessionControlRequestPayload,
        request_id: Option<String>,
    ) -> Result<(), BrokerError> {
        payload.validate()?;
        if payload.action == crate::protocol::ControlAction::Request {
            return self
                .request_suggestion(
                    coordinates,
                    SuggestRequestPayload {
                        fingerprint: payload.fingerprint,
                        explicit: true,
                    },
                    request_id,
                )
                .await;
        }

        let mut state = self.inner.state.lock().await;
        if state.paused {
            return Err(BrokerError::Denied(PolicyReason::Paused));
        }
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        ensure_coordinates(session.coordinates, coordinates)?;
        ensure_fingerprint(session, &payload.fingerprint)?;
        let expected_id = payload
            .suggestion_id
            .as_deref()
            .ok_or(BrokerError::InvalidPayload)?;
        let visible = session.visible.as_ref().ok_or(BrokerError::NoSuggestion)?;
        if visible.payload.suggestion_id != expected_id {
            return Err(BrokerError::Stale);
        }
        if visible.expires_at <= Instant::now() {
            let expired = session.visible.take().expect("visible suggestion checked");
            let _ = session.sink.send(BrokerEvent::SuggestionClear {
                coordinates,
                payload: SuggestionClearPayload {
                    fingerprint: expired.payload.fingerprint,
                    suggestion_id: Some(expired.payload.suggestion_id),
                    reason: ReasonCode::Expired,
                },
                request_id: expired.request_id,
            });
            self.inner.metrics.record_suggestion_expired();
            return Err(BrokerError::NoSuggestion);
        }

        match payload.action {
            crate::protocol::ControlAction::Dismiss => {
                let visible = session.visible.take().expect("visible suggestion checked");
                session.generation = session.generation.wrapping_add(1);
                let _ = session.sink.send(BrokerEvent::SuggestionClear {
                    coordinates,
                    payload: SuggestionClearPayload {
                        fingerprint: visible.payload.fingerprint,
                        suggestion_id: Some(visible.payload.suggestion_id),
                        reason: ReasonCode::Dismissed,
                    },
                    request_id: visible.request_id,
                });
                self.inner.metrics.record_dismissal();
            }
            crate::protocol::ControlAction::AcceptWord
            | crate::protocol::ControlAction::AcceptAll => {
                let visible = session.visible.take().expect("visible suggestion checked");
                let (acceptance, text, remainder) =
                    if payload.action == crate::protocol::ControlAction::AcceptWord {
                        let parts = accept_word(&visible.payload.text);
                        (Acceptance::Word, parts.accepted, parts.remainder)
                    } else {
                        (Acceptance::All, visible.payload.text.clone(), String::new())
                    };
                let commit = CommitPreparePayload {
                    fingerprint: visible.payload.fingerprint.clone(),
                    suggestion_id: visible.payload.suggestion_id.clone(),
                    text,
                    acceptance,
                };
                let suggestion_id = visible.payload.suggestion_id.clone();
                let expires_at = Instant::now() + self.inner.config.commit_result_lease;
                session.pending = Some(PendingCommit {
                    coordinates,
                    fingerprint: visible.payload.fingerprint,
                    suggestion_id: visible.payload.suggestion_id,
                    acceptance,
                    remainder,
                    request_id: request_id.clone(),
                    expires_at,
                });
                if session
                    .sink
                    .send(BrokerEvent::CommitPrepare {
                        coordinates,
                        payload: commit,
                        request_id,
                    })
                    .is_err()
                {
                    session.pending = None;
                    self.inner.metrics.record_commit_failure();
                    return Err(BrokerError::EventSinkClosed);
                }
                self.inner.metrics.record_commit_prepared();
                let broker = self.clone();
                tokio::spawn(async move {
                    time::sleep(broker.inner.config.commit_result_lease).await;
                    broker
                        .expire_pending_commit(coordinates, suggestion_id, expires_at)
                        .await;
                });
            }
            crate::protocol::ControlAction::Request
            | crate::protocol::ControlAction::Pause
            | crate::protocol::ControlAction::Resume
            | crate::protocol::ControlAction::PauseToggle => {
                return Err(BrokerError::InvalidPayload);
            }
        }
        Ok(())
    }

    pub async fn commit_result(
        &self,
        coordinates: Coordinates,
        payload: CommitResultPayload,
    ) -> Result<(), BrokerError> {
        payload.validate()?;
        let mut state = self.inner.state.lock().await;
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        ensure_coordinates(session.coordinates, coordinates)?;
        let pending = session.pending.take().ok_or(BrokerError::NoPendingCommit)?;
        if pending.coordinates != coordinates
            || pending.fingerprint != payload.fingerprint
            || pending.suggestion_id != payload.suggestion_id
        {
            self.inner.metrics.record_commit_failure();
            return Err(BrokerError::Stale);
        }
        if pending.expires_at <= Instant::now() {
            send_pending_clear(session, pending, ReasonCode::Expired);
            self.inner.metrics.record_commit_failure();
            return Err(BrokerError::CommitLeaseExpired);
        }
        validate_commit_authority(&session.authority, payload.status)?;

        match payload.status {
            CommitStatus::Applied => {
                let continuation = validate_partial_continuation(&pending, coordinates, &payload);
                retire_applied_state(session);
                let continuation = match continuation {
                    Ok(continuation) => continuation,
                    Err(error) => {
                        self.inner.metrics.record_commit_failure();
                        return Err(error);
                    }
                };
                self.inner.metrics.record_commit_applied();
                if let Some(continuation) = continuation {
                    session.coordinates = continuation.coordinates;
                    session.context_seen_at_coordinates = false;
                    let suggestion_id = format!("s:{}", uuid::Uuid::new_v4());
                    let text = pending.remainder;
                    let show = SuggestionShowPayload {
                        fingerprint: continuation.fingerprint,
                        suggestion_id: suggestion_id.clone(),
                        accept_word: accept_word(&text).accepted,
                        text,
                        ttl_ms: duration_millis(self.inner.config.suggestion_ttl),
                        provider: self.inner.provider_kind,
                    };
                    session.visible = Some(VisibleSuggestion {
                        payload: show.clone(),
                        expires_at: Instant::now() + self.inner.config.suggestion_ttl,
                        request_id: None,
                    });
                    let _ = session.sink.send(BrokerEvent::SuggestionShow {
                        coordinates: continuation.coordinates,
                        payload: show,
                        request_id: None,
                    });
                    self.inner.metrics.record_suggestion_shown();
                    let broker = self.clone();
                    let generation = session.generation;
                    tokio::spawn(async move {
                        time::sleep(broker.inner.config.suggestion_ttl).await;
                        broker
                            .expire_suggestion(continuation.coordinates, generation, suggestion_id)
                            .await;
                    });
                }
            }
            CommitStatus::DispatchedUnverified => {
                session.context = None;
            }
            CommitStatus::Stale | CommitStatus::Blocked | CommitStatus::Failed => {
                self.inner.metrics.record_commit_failure();
            }
        }
        Ok(())
    }

    pub async fn set_paused(&self, paused: bool) -> bool {
        let mut state = self.inner.state.lock().await;
        transition_paused(&mut state, paused, &self.inner.metrics)
    }

    pub async fn toggle_paused(&self) -> bool {
        let mut state = self.inner.state.lock().await;
        let paused = !state.paused;
        transition_paused(&mut state, paused, &self.inner.metrics)
    }

    pub async fn is_paused(&self) -> bool {
        self.inner.state.lock().await.paused
    }

    pub async fn session_count(&self) -> u64 {
        u64::try_from(self.inner.state.lock().await.sessions.len()).unwrap_or(u64::MAX)
    }

    pub async fn active_locator(&self) -> Option<ActiveLocator> {
        let state = self.inner.state.lock().await;
        if state.paused {
            return None;
        }
        let mut candidates = state.sessions.values().filter_map(|session| {
            let context = session.context.as_ref()?;
            if !context.payload.field.focused {
                return None;
            }
            Some(ActiveLocator {
                session_id: session.coordinates.session_id,
                focus_epoch: session.coordinates.focus_epoch,
                revision: session.coordinates.revision,
                fingerprint: context.payload.fingerprint.clone(),
                suggestion_id: session
                    .visible
                    .as_ref()
                    .filter(|visible| visible.expires_at > Instant::now())
                    .map(|visible| visible.payload.suggestion_id.clone()),
            })
        });
        let only = candidates.next()?;
        if candidates.next().is_some() {
            None
        } else {
            Some(only)
        }
    }

    pub async fn health_snapshot(&self) -> HealthSnapshot {
        HealthSnapshot {
            provider: self.provider_kind(),
            paused: self.is_paused().await,
            sessions: self.session_count().await,
            max_frame_bytes: MAX_FRAME_BYTES,
            metrics: self.inner.metrics.snapshot(),
            active: self.active_locator().await,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub provider: ProviderKind,
    pub paused: bool,
    pub sessions: u64,
    pub max_frame_bytes: usize,
    pub metrics: MetricsSnapshot,
    pub active: Option<ActiveLocator>,
}

fn retire_session(session: &mut SessionState, metrics: &Metrics, reason: ReasonCode) {
    session.generation = session.generation.wrapping_add(1);
    let mut cancelled = false;
    if let Some(cancellation) = session.cancellation.take() {
        cancellation.cancel();
        cancelled = true;
    }
    if let Some(visible) = session.visible.take() {
        let _ = session.sink.send(BrokerEvent::SuggestionClear {
            coordinates: session.coordinates,
            payload: SuggestionClearPayload {
                fingerprint: visible.payload.fingerprint,
                suggestion_id: Some(visible.payload.suggestion_id),
                reason,
            },
            request_id: visible.request_id,
        });
    } else if let Some(pending) = session.pending.take() {
        let _ = session.sink.send(BrokerEvent::SuggestionClear {
            coordinates: pending.coordinates,
            payload: SuggestionClearPayload {
                fingerprint: pending.fingerprint,
                suggestion_id: Some(pending.suggestion_id),
                reason,
            },
            request_id: pending.request_id,
        });
        cancelled = true;
    }
    if session.pending.take().is_some() {
        cancelled = true;
    }
    if cancelled {
        metrics.record_cancellation();
    }
}

fn send_pending_clear(session: &SessionState, pending: PendingCommit, reason: ReasonCode) {
    let _ = session.sink.send(BrokerEvent::SuggestionClear {
        coordinates: pending.coordinates,
        payload: SuggestionClearPayload {
            fingerprint: pending.fingerprint,
            suggestion_id: Some(pending.suggestion_id),
            reason,
        },
        request_id: pending.request_id,
    });
}

fn validate_partial_continuation(
    pending: &PendingCommit,
    coordinates: Coordinates,
    payload: &CommitResultPayload,
) -> Result<Option<PartialContinuation>, BrokerError> {
    match (payload.new_revision, payload.new_fingerprint.as_ref()) {
        (None, None) => Ok(None),
        (Some(new_revision), Some(new_fingerprint))
            if pending.acceptance == Acceptance::Word && !pending.remainder.is_empty() =>
        {
            if new_revision <= coordinates.revision {
                Err(BrokerError::Stale)
            } else {
                Ok(Some(PartialContinuation {
                    coordinates: Coordinates {
                        revision: new_revision,
                        ..coordinates
                    },
                    fingerprint: new_fingerprint.clone(),
                }))
            }
        }
        _ => Err(BrokerError::InvalidPayload),
    }
}

fn retire_applied_state(session: &mut SessionState) {
    session.generation = session.generation.wrapping_add(1);
    if let Some(cancellation) = session.cancellation.take() {
        cancellation.cancel();
    }
    session.context = None;
    session.visible = None;
}

fn transition_paused(state: &mut BrokerState, paused: bool, metrics: &Metrics) -> bool {
    if state.paused == paused {
        return state.paused;
    }
    state.paused = paused;
    if paused {
        for session in state.sessions.values_mut() {
            retire_session(session, metrics, ReasonCode::Paused);
            session.context = None;
        }
    }
    state.paused
}

fn ensure_newer_context(
    session: &SessionState,
    coordinates: Coordinates,
) -> Result<(), BrokerError> {
    if coordinates.focus_epoch < session.coordinates.focus_epoch
        || (coordinates.focus_epoch == session.coordinates.focus_epoch
            && coordinates.revision < session.coordinates.revision)
        || (coordinates.focus_epoch == session.coordinates.focus_epoch
            && coordinates.revision == session.coordinates.revision
            && session.context_seen_at_coordinates)
    {
        Err(BrokerError::Stale)
    } else {
        Ok(())
    }
}

fn ensure_coordinates(expected: Coordinates, actual: Coordinates) -> Result<(), BrokerError> {
    if expected == actual {
        Ok(())
    } else {
        Err(BrokerError::Stale)
    }
}

fn ensure_fingerprint(session: &SessionState, fingerprint: &str) -> Result<(), BrokerError> {
    let current = session
        .context
        .as_ref()
        .map(|context| context.payload.fingerprint.as_str())
        .or_else(|| {
            session
                .visible
                .as_ref()
                .map(|visible| visible.payload.fingerprint.as_str())
        });
    if current == Some(fingerprint) {
        Ok(())
    } else {
        Err(BrokerError::Stale)
    }
}

fn validate_commit_authority(
    authority: &SessionAuthority,
    status: CommitStatus,
) -> Result<(), BrokerError> {
    let allowed = match status {
        CommitStatus::Applied => {
            matches!(
                authority.adapter_kind,
                AdapterKind::Browser | AdapterKind::Obsidian
            ) && authority.capabilities.contains(&Capability::CommitApplied)
        }
        CommitStatus::DispatchedUnverified => {
            matches!(
                authority.adapter_kind,
                AdapterKind::Browser | AdapterKind::Fcitx
            ) && authority
                .capabilities
                .contains(&Capability::CommitDispatchedUnverified)
        }
        CommitStatus::Stale | CommitStatus::Blocked | CommitStatus::Failed => true,
    };
    if allowed {
        Ok(())
    } else {
        Err(BrokerError::InvalidCapability)
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

const fn restrictive_activation(configured: Activation, claimed: Activation) -> Activation {
    match (configured, claimed) {
        (Activation::Never, _) | (_, Activation::Never) => Activation::Never,
        (Activation::Manual, _) | (_, Activation::Manual) => Activation::Manual,
        (Activation::Always, Activation::Always) => Activation::Always,
    }
}

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("commit_lease_expired")]
    CommitLeaseExpired,
    #[error("denied:{0:?}")]
    Denied(PolicyReason),
    #[error("invalid_capability")]
    InvalidCapability,
    #[error("invalid_payload")]
    InvalidPayload,
    #[error("event_sink_closed")]
    EventSinkClosed,
    #[error("manual_required")]
    ManualRequired,
    #[error("no_context")]
    NoContext,
    #[error("no_pending_commit")]
    NoPendingCommit,
    #[error("no_suggestion")]
    NoSuggestion,
    #[error("protocol")]
    Protocol(#[from] crate::protocol::ProtocolError),
    #[error("session_already_open")]
    SessionAlreadyOpen,
    #[error("stale")]
    Stale,
    #[error("unknown_session")]
    UnknownSession,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use async_trait::async_trait;
    use tokio::sync::{Barrier, mpsc};
    use tokio::time::{Duration, sleep, timeout};
    use tokio_util::sync::CancellationToken;

    use super::{
        Broker, BrokerConfig, BrokerError, BrokerEvent, ContextOutcome, SessionAuthority,
        validate_commit_authority,
    };
    use crate::protocol::{
        Activation, AdapterKind, Capability, CommitResultPayload, CommitStatus,
        ContextChangedPayload, ControlAction, Coordinates, FieldDescriptor, FieldPurpose,
        OffsetUnit, ProviderKind, Selection, SessionControlRequestPayload, SessionId,
        SessionOpenPayload, SuggestRequestPayload, TargetDescriptor, TargetKind,
    };
    use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};

    struct CountingProvider {
        calls: AtomicU64,
        bytes: AtomicU64,
        delay: Duration,
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

    #[test]
    fn commit_status_requires_matching_adapter_kind_and_declared_capability() {
        let browser_dispatch = SessionAuthority {
            adapter_kind: AdapterKind::Browser,
            capabilities: vec![Capability::CommitDispatchedUnverified],
        };
        assert!(
            validate_commit_authority(&browser_dispatch, CommitStatus::DispatchedUnverified)
                .is_ok()
        );
        assert!(matches!(
            validate_commit_authority(&browser_dispatch, CommitStatus::Applied),
            Err(BrokerError::InvalidCapability)
        ));

        let browser_applied = SessionAuthority {
            adapter_kind: AdapterKind::Browser,
            capabilities: vec![Capability::CommitApplied],
        };
        assert!(validate_commit_authority(&browser_applied, CommitStatus::Applied).is_ok());

        let browser_undeclared = SessionAuthority {
            adapter_kind: AdapterKind::Browser,
            capabilities: Vec::new(),
        };
        assert!(matches!(
            validate_commit_authority(&browser_undeclared, CommitStatus::DispatchedUnverified),
            Err(BrokerError::InvalidCapability)
        ));

        let obsidian_wrong_status = SessionAuthority {
            adapter_kind: AdapterKind::Obsidian,
            capabilities: vec![Capability::CommitDispatchedUnverified],
        };
        assert!(matches!(
            validate_commit_authority(&obsidian_wrong_status, CommitStatus::DispatchedUnverified,),
            Err(BrokerError::InvalidCapability)
        ));

        let fcitx_dispatch = SessionAuthority {
            adapter_kind: AdapterKind::Fcitx,
            capabilities: vec![Capability::CommitDispatchedUnverified],
        };
        assert!(
            validate_commit_authority(&fcitx_dispatch, CommitStatus::DispatchedUnverified).is_ok()
        );
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

    fn coordinates(session_id: SessionId, revision: u64) -> Coordinates {
        Coordinates {
            session_id,
            focus_epoch: 1,
            revision,
        }
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

    async fn setup(
        provider: std::sync::Arc<dyn CompletionProvider>,
        config: BrokerConfig,
    ) -> (Broker, SessionId, mpsc::UnboundedReceiver<BrokerEvent>) {
        let broker = Broker::new(provider, config);
        let session_id = SessionId::new();
        let (sink, receiver) = mpsc::unbounded_channel();
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
                    adapter_kind: AdapterKind::Browser,
                    capabilities: vec![
                        Capability::Context,
                        Capability::Suggestion,
                        Capability::CommitApplied,
                    ],
                },
                sink,
            )
            .await
            .expect("open session");
        (broker, session_id, receiver)
    }

    async fn prepare_commit_result_case(
        broker: &Broker,
        session_id: SessionId,
        events: &mut mpsc::UnboundedReceiver<BrokerEvent>,
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
        assert!(broker.metrics().snapshot().stale_results >= 100);
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
        let (sink, mut events) = mpsc::unbounded_channel();
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
                    adapter_kind: AdapterKind::Browser,
                    capabilities: vec![Capability::Context, Capability::Suggestion],
                },
                sink,
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
        let (second_sink, _second_events) = mpsc::unbounded_channel();
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
                    adapter_kind: AdapterKind::Browser,
                    capabilities: vec![Capability::Context, Capability::Suggestion],
                },
                second_sink,
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
                        new_revision: None,
                        new_fingerprint: None,
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
                    new_revision: None,
                    new_fingerprint: None,
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
                    new_revision: None,
                    new_fingerprint: None,
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
    async fn applied_word_rejects_non_increasing_rebind_before_success_metric() {
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

        assert!(matches!(
            broker
                .commit_result(
                    coordinates(session_id, 1),
                    CommitResultPayload {
                        fingerprint: update.fingerprint,
                        suggestion_id,
                        status: CommitStatus::Applied,
                        new_revision: Some(1),
                        new_fingerprint: Some("fingerprint_000000000002".to_owned()),
                    },
                )
                .await,
            Err(BrokerError::Stale)
        ));

        assert_pre_mutation_state_retired(&broker, session_id).await;
        assert!(events.try_recv().is_err());
        let metrics = broker.metrics().snapshot();
        assert_eq!(metrics.commits_applied, 0);
        assert_eq!(metrics.commit_failures, 1);
    }

    #[tokio::test]
    async fn applied_word_with_valid_rebind_installs_only_partial_remainder() {
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
                    new_revision: Some(2),
                    new_fingerprint: Some("fingerprint_000000000002".to_owned()),
                },
            )
            .await
            .expect("valid partial rebind");

        let continued = events.recv().await.expect("continued suggestion");
        let BrokerEvent::SuggestionShow {
            coordinates: continued_coordinates,
            payload,
            ..
        } = continued
        else {
            panic!("expected continued suggestion")
        };
        assert_eq!(continued_coordinates, coordinates(session_id, 2));
        assert_eq!(payload.fingerprint, "fingerprint_000000000002");
        assert_eq!(payload.text, " 1");
        assert_eq!(payload.accept_word, " 1");
        let state = broker.inner.state.lock().await;
        let session = state.sessions.get(&session_id).expect("session");
        assert_eq!(session.coordinates, coordinates(session_id, 2));
        assert!(session.context.is_none());
        assert!(session.pending.is_none());
        assert!(session.visible.is_some());
        assert!(!session.context_seen_at_coordinates);
        drop(state);
        let metrics = broker.metrics().snapshot();
        assert_eq!(metrics.commits_applied, 1);
        assert_eq!(metrics.commit_failures, 0);
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
                        new_revision: None,
                        new_fingerprint: None,
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
                        new_revision: None,
                        new_fingerprint: None,
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
                        new_revision: None,
                        new_fingerprint: None,
                    },
                )
                .await,
            Err(BrokerError::NoPendingCommit)
        ));
    }
}
