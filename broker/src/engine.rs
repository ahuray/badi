use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tokio::sync::{Mutex, Semaphore, broadcast, mpsc, oneshot};
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::control_plane::{ControlPlane, ControlPlaneError, ControlPlaneSnapshot};
use crate::metrics::{Metrics, MetricsSnapshot};
use crate::personalization::{
    PersonalizationProvider, PersonalizationSignal, PersonalizationStoreError,
};
use crate::policy::{PolicyDecision, PolicyInput, PolicyReason, evaluate};
use crate::protocol::{
    Acceptance, Activation, ActiveLocator, AdapterKind, AuthorityChangedPayload, Capability,
    CommitPreparePayload, CommitResultPayload, CommitStatus, ContextChangedPayload, Coordinates,
    DEFAULT_SUGGESTION_TTL_MS, MAX_FRAME_BYTES, MAX_SAFE_COUNTER, MessageType,
    PolicyResolutionReason, PolicyStatusPayload, ProviderKind, ReasonCode,
    SessionControlRequestPayload, SessionId, SessionOpenPayload, SuggestCancelPayload,
    SuggestRequestPayload, SuggestionClearPayload, SuggestionShowPayload, TargetDescriptor,
    WireEnvelope,
};
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
use crate::segment::{OutputError, accept_word, sanitize_suggestion, validate_suggestion_shape};
use crate::settings::{PrivateStorageError, SettingsStoreError, SettingsV1, StableIdentity};

/// Default broker-local time allowed for an adapter to report a commit result.
pub const DEFAULT_COMMIT_RESULT_LEASE_MS: u64 = 1_500;
/// Hard ceiling for the broker-local commit-result lease.
pub const MAX_COMMIT_RESULT_LEASE_MS: u64 = 5_000;
/// Default silence window before retained context authority is revoked.
pub const DEFAULT_CONTEXT_AUTHORITY_LEASE_MS: u64 = 3_000;
const MIN_CONTEXT_AUTHORITY_LEASE_MS: u64 = 500;
const MAX_CONTEXT_AUTHORITY_LEASE_MS: u64 = 10_000;
const OUTCOME_QUEUE_CAPACITY: usize = 256;
const OUTCOME_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const PERSONALIZATION_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
/// Default number of provider generations allowed across the entire broker.
pub const DEFAULT_PROVIDER_CONCURRENCY: usize = 4;
/// Hard upper bound even when a caller supplies a larger configuration value.
pub const MAX_PROVIDER_CONCURRENCY: usize = 16;
/// Maximum receiver-local age from an accepted request to provider completion.
pub const MAX_GENERATION_TIMEOUT_MS: u64 = 600;

#[derive(Clone, Copy, Debug)]
pub struct BrokerConfig {
    pub debounce: Duration,
    pub provider_timeout: Duration,
    /// Includes broker debounce; late output is never displayed with a fresh TTL.
    pub generation_timeout: Duration,
    /// Maximum provider generations admitted across all broker connections.
    pub provider_concurrency: usize,
    pub suggestion_ttl: Duration,
    /// Receiver-local silence lease for retained context and derived authority.
    pub context_authority_lease: Duration,
    /// Receiver-local lease; adapters must report the authorized commit before it expires.
    pub commit_result_lease: Duration,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            // Adapters own user-idle acquisition timing. A second production
            // debounce here only consumes the single input-to-visible budget.
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_millis(1_300),
            generation_timeout: Duration::from_millis(MAX_GENERATION_TIMEOUT_MS),
            provider_concurrency: DEFAULT_PROVIDER_CONCURRENCY,
            suggestion_ttl: Duration::from_millis(DEFAULT_SUGGESTION_TTL_MS),
            context_authority_lease: Duration::from_millis(DEFAULT_CONTEXT_AUTHORITY_LEASE_MS),
            commit_result_lease: Duration::from_millis(DEFAULT_COMMIT_RESULT_LEASE_MS),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionAuthority {
    pub adapter_kind: AdapterKind,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone)]
pub struct BrokerEventSink {
    sender: mpsc::Sender<BrokerEvent>,
    connection_lifetime: CancellationToken,
}

impl BrokerEventSink {
    #[must_use]
    pub fn new(sender: mpsc::Sender<BrokerEvent>, connection_lifetime: CancellationToken) -> Self {
        Self {
            sender,
            connection_lifetime,
        }
    }

    fn send(&self, event: BrokerEvent) -> Result<(), BrokerError> {
        self.sender.try_send(event).map_err(|_| {
            // Full and closed both mean this connection cannot reliably observe
            // revocation. End the connection instead of leaving stale authority
            // rendered in a live adapter.
            self.connection_lifetime.cancel();
            BrokerError::EventSinkClosed
        })
    }
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
    control_plane: Option<Arc<ControlPlane>>,
    control_plane_mutation: Mutex<()>,
    provider: Arc<dyn CompletionProvider>,
    provider_admissions: Arc<Semaphore>,
    provider_kind: ProviderKind,
    config: BrokerConfig,
    metrics: Arc<Metrics>,
    shutdown: CancellationToken,
    authority_events: broadcast::Sender<AuthorityChangedPayload>,
    outcome_recorder: Option<OutcomeRecorder>,
    started: Instant,
    state: Mutex<BrokerState>,
}

#[derive(Default)]
struct BrokerState {
    paused: bool,
    control_plane_condition: ControlPlaneCondition,
    control_plane_mutation_in_progress: bool,
    authority_epoch: u64,
    settings_revision: u64,
    settings: Option<SettingsV1>,
    policy_clients: HashMap<String, Option<u64>>,
    sessions: HashMap<SessionId, SessionState>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ControlPlaneCondition {
    #[default]
    Healthy,
    Recoverable,
    RestartRequired,
}

impl ControlPlaneCondition {
    const fn is_degraded(self) -> bool {
        !matches!(self, Self::Healthy)
    }
}

struct SessionState {
    coordinates: Coordinates,
    target: SessionOpenPayload,
    authority: SessionAuthority,
    context: Option<StoredContext>,
    context_seen_at_coordinates: bool,
    context_lease_generation: u64,
    context_lease_cancellation: Option<CancellationToken>,
    generation: u64,
    cancellation: Option<CancellationToken>,
    visible: Option<VisibleSuggestion>,
    pending: Option<PendingCommit>,
    sink: BrokerEventSink,
}

#[derive(Clone)]
struct StoredContext {
    payload: ContextChangedPayload,
}

struct GenerationResultContext {
    coordinates: Coordinates,
    fingerprint: String,
    generation: u64,
    request_id: Option<String>,
    deadline: Instant,
    before: String,
    after: String,
}

struct VisibleSuggestion {
    payload: SuggestionShowPayload,
    expires_at: Instant,
    request_id: Option<String>,
    aggregate_day: Option<u64>,
}

struct PendingCommit {
    coordinates: Coordinates,
    fingerprint: String,
    suggestion_id: String,
    request_id: Option<String>,
    expires_at: Instant,
}

enum SettingsReplaceOutcome {
    Updated(ControlPlaneSnapshot),
    CommittedDegraded {
        settings: SettingsV1,
        error: ControlPlaneError,
    },
    CommitUnknown(ControlPlaneError),
    RejectedDegraded(ControlPlaneError),
    Rejected(ControlPlaneError),
}

#[derive(Clone)]
struct OutcomeRecorder {
    sender: std_mpsc::SyncSender<OutcomeCommand>,
    available: Arc<AtomicBool>,
    dropped_signals: Arc<AtomicU64>,
    write_failures: Arc<AtomicU64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutcomeRecorderHealth {
    pub available: bool,
    pub dropped_signals: u64,
    pub write_failures: u64,
}

enum OutcomeCommand {
    Signal {
        expected_settings_revision: u64,
        event_day: u64,
        identity: StableIdentity,
        provider: PersonalizationProvider,
        signal: PersonalizationSignal,
    },
    Clear {
        response: oneshot::Sender<Result<(bool, ControlPlaneSnapshot), ControlPlaneError>>,
    },
    Snapshot {
        response: oneshot::Sender<Result<ControlPlaneSnapshot, ControlPlaneError>>,
    },
    Flush {
        response: oneshot::Sender<()>,
    },
}

impl OutcomeRecorder {
    fn new(control_plane: Arc<ControlPlane>) -> Result<Self, std::io::Error> {
        let (sender, receiver) = std_mpsc::sync_channel(OUTCOME_QUEUE_CAPACITY);
        let available = Arc::new(AtomicBool::new(true));
        let dropped_signals = Arc::new(AtomicU64::new(0));
        let write_failures = Arc::new(AtomicU64::new(0));
        let worker_available = Arc::clone(&available);
        let worker_dropped = Arc::clone(&dropped_signals);
        let worker_failures = Arc::clone(&write_failures);
        std::thread::Builder::new()
            .name("badi-outcomes".to_owned())
            .spawn(move || {
                let mut reported_dropped = 0;
                let mut next_sweep = Instant::now() + PERSONALIZATION_SWEEP_INTERVAL;
                loop {
                    let wait = next_sweep.saturating_duration_since(Instant::now());
                    let command = match receiver.recv_timeout(wait) {
                        Ok(command) => command,
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {
                            sweep_personalization_retention(&control_plane, &worker_failures);
                            next_sweep = Instant::now() + PERSONALIZATION_SWEEP_INTERVAL;
                            continue;
                        }
                        Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    match command {
                        OutcomeCommand::Signal {
                            expected_settings_revision,
                            event_day,
                            identity,
                            provider,
                            signal,
                        } => {
                            match control_plane.record_signal_at_settings_revision(
                                expected_settings_revision,
                                event_day,
                                identity,
                                provider,
                                signal,
                            ) {
                                Ok(mutation) if mutation.signal_dropped => {
                                    worker_dropped.fetch_add(1, Ordering::Relaxed);
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    worker_failures.fetch_add(1, Ordering::Relaxed);
                                    eprintln!(
                                        "badi-broker: outcome aggregate write failed: {error}"
                                    );
                                }
                            }
                        }
                        OutcomeCommand::Clear { response } => {
                            let result = control_plane.clear_personalization().and_then(|mutation| {
                                control_plane
                                    .snapshot()
                                    .map(|snapshot| (mutation.changed, snapshot))
                            });
                            let _ = response.send(result);
                        }
                        OutcomeCommand::Snapshot { response } => {
                            let _ = response.send(control_plane.snapshot());
                        }
                        OutcomeCommand::Flush { response } => {
                            let _ = response.send(());
                        }
                    }
                    let dropped = worker_dropped.load(Ordering::Relaxed);
                    if dropped > reported_dropped {
                        let rejected = dropped - reported_dropped;
                        reported_dropped = dropped;
                        eprintln!(
                            "badi-broker: dropped {rejected} outcome aggregate signal(s) because the recorder queue or bounded store could not admit them"
                        );
                    }
                    if Instant::now() >= next_sweep {
                        sweep_personalization_retention(&control_plane, &worker_failures);
                        next_sweep = Instant::now() + PERSONALIZATION_SWEEP_INTERVAL;
                    }
                }
                worker_available.store(false, Ordering::Relaxed);
            })?;
        Ok(Self {
            sender,
            available,
            dropped_signals,
            write_failures,
        })
    }

    fn try_signal(&self, command: OutcomeCommand) -> bool {
        match self.sender.try_send(command) {
            Ok(()) => true,
            Err(std_mpsc::TrySendError::Full(_)) => {
                self.dropped_signals.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(std_mpsc::TrySendError::Disconnected(_)) => {
                self.available.store(false, Ordering::Relaxed);
                self.dropped_signals.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    fn health(&self) -> OutcomeRecorderHealth {
        OutcomeRecorderHealth {
            available: self.available.load(Ordering::Relaxed),
            dropped_signals: self.dropped_signals.load(Ordering::Relaxed),
            write_failures: self.write_failures.load(Ordering::Relaxed),
        }
    }

    async fn clear(&self) -> Result<(bool, ControlPlaneSnapshot), BrokerError> {
        let (response, receiver) = oneshot::channel();
        let sender = self.sender.clone();
        tokio::task::spawn_blocking(move || sender.send(OutcomeCommand::Clear { response }))
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(|_| BrokerError::ControlPlaneUnavailable)?;
        receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)
    }

    async fn snapshot(&self) -> Result<ControlPlaneSnapshot, BrokerError> {
        let (response, receiver) = oneshot::channel();
        let sender = self.sender.clone();
        tokio::task::spawn_blocking(move || sender.send(OutcomeCommand::Snapshot { response }))
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(|_| BrokerError::ControlPlaneUnavailable)?;
        receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)
    }

    async fn flush(&self) -> Result<(), BrokerError> {
        let (response, receiver) = oneshot::channel();
        let sender = self.sender.clone();
        tokio::task::spawn_blocking(move || sender.send(OutcomeCommand::Flush { response }))
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(|_| BrokerError::ControlPlaneUnavailable)?;
        receiver.await.map_err(|_| BrokerError::ControlPlaneTask)
    }
}

fn sweep_personalization_retention(control_plane: &ControlPlane, write_failures: &AtomicU64) {
    if let Err(error) = control_plane.reconcile_personalization_now() {
        write_failures.fetch_add(1, Ordering::Relaxed);
        eprintln!("badi-broker: periodic personalization retention reconciliation failed: {error}");
    }
}

impl Broker {
    #[must_use]
    pub fn new(provider: Arc<dyn CompletionProvider>, config: BrokerConfig) -> Self {
        Self::build(provider, config, None, None, None)
    }

    pub fn with_control_plane(
        provider: Arc<dyn CompletionProvider>,
        config: BrokerConfig,
        control_plane: Arc<ControlPlane>,
    ) -> Result<Self, BrokerError> {
        let settings = control_plane.snapshot()?.settings;
        let outcome_recorder = OutcomeRecorder::new(Arc::clone(&control_plane))?;
        Ok(Self::build(
            provider,
            config,
            Some(control_plane),
            Some(settings),
            Some(outcome_recorder),
        ))
    }

    fn build(
        provider: Arc<dyn CompletionProvider>,
        config: BrokerConfig,
        control_plane: Option<Arc<ControlPlane>>,
        settings: Option<SettingsV1>,
        outcome_recorder: Option<OutcomeRecorder>,
    ) -> Self {
        let provider_kind = provider.kind();
        let config = BrokerConfig {
            provider_concurrency: config
                .provider_concurrency
                .clamp(1, MAX_PROVIDER_CONCURRENCY),
            suggestion_ttl: config
                .suggestion_ttl
                .clamp(Duration::from_millis(1), Duration::from_millis(600)),
            generation_timeout: config.generation_timeout.clamp(
                Duration::from_millis(1),
                Duration::from_millis(MAX_GENERATION_TIMEOUT_MS),
            ),
            commit_result_lease: config.commit_result_lease.clamp(
                Duration::from_millis(1),
                Duration::from_millis(MAX_COMMIT_RESULT_LEASE_MS),
            ),
            context_authority_lease: config.context_authority_lease.clamp(
                Duration::from_millis(MIN_CONTEXT_AUTHORITY_LEASE_MS),
                Duration::from_millis(MAX_CONTEXT_AUTHORITY_LEASE_MS),
            ),
            ..config
        };
        let provider_admissions = Arc::new(Semaphore::new(config.provider_concurrency));
        let (authority_events, _) = broadcast::channel(32);
        Self {
            inner: Arc::new(BrokerInner {
                control_plane,
                control_plane_mutation: Mutex::new(()),
                provider,
                provider_admissions,
                provider_kind,
                config,
                metrics: Arc::new(Metrics::default()),
                shutdown: CancellationToken::new(),
                authority_events,
                outcome_recorder,
                started: Instant::now(),
                state: Mutex::new(BrokerState {
                    settings_revision: settings.as_ref().map_or(0, |value| value.revision),
                    settings,
                    ..BrokerState::default()
                }),
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
        sink: BrokerEventSink,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_coordinate_bounds(coordinates.focus_epoch, coordinates.revision)?;
        payload.target.validate()?;
        if !authority.capabilities.contains(&Capability::Context)
            || !authority.capabilities.contains(&Capability::Suggestion)
        {
            return Err(BrokerError::InvalidCapability);
        }
        let mut state = self.inner.state.lock().await;
        if self.inner.shutdown.is_cancelled() {
            return Err(BrokerError::ShuttingDown);
        }
        if state.sessions.contains_key(&coordinates.session_id) {
            return Err(BrokerError::SessionAlreadyOpen);
        }
        if let Some(settings) = state.settings.as_ref() {
            if !authority.capabilities.contains(&Capability::Policy) {
                return Err(BrokerError::InvalidCapability);
            }
            let policy = policy_status(
                runtime_control_paused(&state),
                state.authority_epoch,
                settings,
                &payload.target,
            );
            if payload.activation != Activation::Always
                || !policy.context_allowed
                || !policy.display_allowed
                || !policy.suggestions_allowed
            {
                return Err(BrokerError::Denied(PolicyReason::PolicyNever));
            }
        }
        state.sessions.insert(
            coordinates.session_id,
            SessionState {
                coordinates,
                target: payload,
                authority,
                context: None,
                context_seen_at_coordinates: false,
                context_lease_generation: 0,
                context_lease_cancellation: None,
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
        revoke_context_authority(&mut session, &self.inner.metrics, ReasonCode::SessionClosed);
        Ok(())
    }

    pub async fn close_owned_sessions(&self, session_ids: &[SessionId]) {
        let mut state = self.inner.state.lock().await;
        for session_id in session_ids {
            if let Some(mut session) = state.sessions.remove(session_id) {
                revoke_context_authority(
                    &mut session,
                    &self.inner.metrics,
                    ReasonCode::SessionClosed,
                );
            }
        }
    }

    /// Retires every session and cancels all provider work before server shutdown.
    pub async fn shutdown(&self) {
        // Closing admission is terminal and nonblocking: no request racing
        // shutdown can acquire new provider capacity after this point.
        self.inner.provider_admissions.close();
        self.inner.shutdown.cancel();
        let mut state = self.inner.state.lock().await;
        for session in state.sessions.values_mut() {
            revoke_context_authority(session, &self.inner.metrics, ReasonCode::SessionClosed);
        }
        state.sessions.clear();
        drop(state);
        if let Some(recorder) = self.inner.outcome_recorder.as_ref() {
            match time::timeout(OUTCOME_FLUSH_TIMEOUT, recorder.flush()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    eprintln!("badi-broker: outcome aggregate flush failed: {error}");
                }
                Err(_) => {
                    eprintln!("badi-broker: outcome aggregate flush timed out");
                }
            }
        }
    }

    fn renew_context_authority_lease(&self, session: &mut SessionState) {
        invalidate_context_authority_lease(session);
        let generation = session.context_lease_generation;
        let cancellation = CancellationToken::new();
        session.context_lease_cancellation = Some(cancellation.clone());
        let session_id = session.coordinates.session_id;
        let lease = self.inner.config.context_authority_lease;
        let broker = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = time::sleep(lease) => {
                    broker
                        .expire_context_authority(session_id, generation)
                        .await;
                }
                () = cancellation.cancelled() => {}
            }
        });
    }

    async fn expire_context_authority(&self, session_id: SessionId, generation: u64) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&session_id) else {
            return;
        };
        if session.context_lease_generation != generation
            || session.context_lease_cancellation.is_none()
        {
            return;
        }
        revoke_context_authority(session, &self.inner.metrics, ReasonCode::Expired);
    }

    pub async fn update_context(
        &self,
        coordinates: Coordinates,
        mut payload: ContextChangedPayload,
    ) -> Result<ContextOutcome, BrokerError> {
        crate::protocol::validate_coordinate_bounds(coordinates.focus_epoch, coordinates.revision)?;
        let mut state = self.inner.state.lock().await;
        let runtime_paused = runtime_control_paused(&state);
        let settings = state.settings.clone();
        let paused = runtime_paused || settings.as_ref().is_some_and(|value| value.paused);
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        if let Some(settings) = settings.as_ref() {
            if !settings_allows_data(runtime_paused, settings, &session.target.target) {
                return Err(BrokerError::Denied(PolicyReason::PolicyNever));
            }
        }
        ensure_newer_context(session, coordinates)?;
        // Apply broker-owned activation before validating content. A restrictive
        // session must reject non-empty context even when an adapter claims a
        // more permissive activation in this individual update.
        payload.activation = restrictive_activation(session.target.activation, payload.activation);
        payload.validate()?;
        self.inner.metrics.record_context_update();
        revoke_context_authority(session, &self.inner.metrics, ReasonCode::Superseded);
        session.coordinates = coordinates;
        session.context_seen_at_coordinates = true;

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
                self.renew_context_authority_lease(session);
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

    #[allow(clippy::too_many_lines)]
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

        let (
            provider_request,
            suggestion_context,
            generation_deadline,
            cancellation,
            generation,
            provider_permit,
        ) = {
            let mut state = self.inner.state.lock().await;
            let runtime_paused = runtime_control_paused(&state);
            let settings = state.settings.clone();
            let paused = runtime_paused || settings.as_ref().is_some_and(|value| value.paused);
            let session = state
                .sessions
                .get_mut(&coordinates.session_id)
                .ok_or(BrokerError::UnknownSession)?;
            if settings.as_ref().is_some_and(|settings| {
                !settings_allows_data(runtime_paused, settings, &session.target.target)
            }) {
                return Err(BrokerError::Denied(PolicyReason::PolicyNever));
            }
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

            // Supersession cancels any older generation before capacity is
            // tested. A saturated broker therefore fails closed without
            // allowing an obsolete suggestion to remain eligible.
            retire_session(session, &self.inner.metrics, ReasonCode::Superseded);
            let provider_permit = Arc::clone(&self.inner.provider_admissions)
                .try_acquire_owned()
                .map_err(|_| {
                    self.inner.metrics.record_provider_error();
                    BrokerError::ProviderBusy
                })?;
            session.generation = session.generation.wrapping_add(1);
            let generation = session.generation;
            let cancellation = CancellationToken::new();
            session.cancellation = Some(cancellation.clone());
            self.renew_context_authority_lease(session);
            let suggestion_context = (
                context.payload.before.clone(),
                context.payload.after.clone(),
            );
            (
                ProviderRequest {
                    before: context.payload.before,
                    after: context.payload.after,
                    language: context.payload.language,
                },
                suggestion_context,
                Instant::now() + self.inner.config.generation_timeout,
                cancellation,
                generation,
                provider_permit,
            )
        };

        let broker = self.clone();
        tokio::spawn(async move {
            // Admission occurs before spawning, so there can never be more
            // generation tasks than permits. The permit is released on every
            // return path, including cancellation and timeout.
            let _provider_permit = provider_permit;
            if broker.inner.config.debounce != Duration::ZERO {
                tokio::select! {
                    () = time::sleep(broker.inner.config.debounce) => {}
                    () = cancellation.cancelled() => return,
                    () = broker.inner.shutdown.cancelled() => return,
                }
            }
            if cancellation.is_cancelled() || broker.inner.shutdown.is_cancelled() {
                return;
            }
            broker
                .inner
                .metrics
                .record_provider_call(provider_request.byte_len());
            let provider_deadline = Instant::now() + broker.inner.config.provider_timeout;
            let effective_deadline = std::cmp::min(generation_deadline, provider_deadline);
            let result = tokio::select! {
                () = cancellation.cancelled() => return,
                () = broker.inner.shutdown.cancelled() => return,
                result = time::timeout_at(
                    effective_deadline.into(),
                    broker
                        .inner
                        .provider
                        .complete(provider_request, cancellation.clone()),
                ) => result,
            };
            broker
                .finish_generation(
                    GenerationResultContext {
                        coordinates,
                        fingerprint: payload.fingerprint,
                        generation,
                        request_id,
                        deadline: generation_deadline,
                        before: suggestion_context.0,
                        after: suggestion_context.1,
                    },
                    cancellation,
                    result,
                )
                .await;
        });
        Ok(())
    }

    // Keeping this result-state transition contiguous makes cancellation and
    // stale-authority auditing clearer than splitting coupled branches.
    #[allow(clippy::too_many_lines)]
    async fn finish_generation(
        &self,
        context: GenerationResultContext,
        cancellation: CancellationToken,
        result: Result<Result<Option<String>, ProviderError>, time::error::Elapsed>,
    ) {
        let GenerationResultContext {
            coordinates,
            fingerprint,
            generation,
            request_id,
            deadline,
            before,
            after,
        } = context;
        if Instant::now() >= deadline {
            cancellation.cancel();
            self.inner.metrics.record_provider_error();
            self.clear_failed_generation(
                coordinates,
                &fingerprint,
                generation,
                request_id,
                ReasonCode::ProviderTimeout,
            )
            .await;
            return;
        }
        let timed_out = result.is_err();
        let output = match result {
            Ok(Ok(Some(raw))) => {
                self.inner.metrics.record_provider_output(raw.len());
                sanitize_suggestion(&raw)
            }
            Ok(Ok(None)) => Err(OutputError::Empty),
            Ok(Err(ProviderError::Cancelled)) => return,
            Ok(Err(ProviderError::Unavailable)) | Err(_) => {
                if timed_out {
                    cancellation.cancel();
                }
                self.inner.metrics.record_provider_error();
                let reason = if timed_out {
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
        if validate_suggestion_shape(&before, &after, &text).is_err() {
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
        }

        let mut state = self.inner.state.lock().await;
        if Instant::now() >= deadline {
            drop(state);
            cancellation.cancel();
            self.inner.metrics.record_provider_error();
            self.clear_failed_generation(
                coordinates,
                &fingerprint,
                generation,
                request_id,
                ReasonCode::ProviderTimeout,
            )
            .await;
            return;
        }
        let runtime_paused = runtime_control_paused(&state);
        let settings = state.settings.clone();
        let settings_revision = state.settings_revision;
        if runtime_paused || settings.as_ref().is_some_and(|value| value.paused) {
            self.inner.metrics.record_stale_result();
            return;
        }
        let Some(session) = state.sessions.get_mut(&coordinates.session_id) else {
            self.inner.metrics.record_stale_result();
            return;
        };
        if settings.as_ref().is_some_and(|settings| {
            !settings_allows_data(runtime_paused, settings, &session.target.target)
        }) {
            self.inner.metrics.record_stale_result();
            return;
        }
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
        if Instant::now() >= deadline {
            cancellation.cancel();
            self.inner.metrics.record_provider_error();
            session.cancellation = None;
            let _ = session.sink.send(BrokerEvent::SuggestionClear {
                coordinates,
                payload: SuggestionClearPayload {
                    fingerprint: payload.fingerprint,
                    suggestion_id: None,
                    reason: ReasonCode::ProviderTimeout,
                },
                request_id,
            });
            return;
        }
        if session
            .sink
            .send(BrokerEvent::SuggestionShow {
                coordinates,
                payload: payload.clone(),
                request_id: request_id.clone(),
            })
            .is_err()
        {
            return;
        }
        let target = session.target.target.clone();
        self.inner.metrics.record_suggestion_shown();
        let aggregate_day = current_unix_day().filter(|event_day| {
            self.queue_outcome(
                &target,
                settings_revision,
                *event_day,
                PersonalizationSignal::Shown,
            )
        });
        session.visible = Some(VisibleSuggestion {
            payload,
            expires_at,
            request_id,
            aggregate_day,
        });
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
        let runtime_paused = runtime_control_paused(&state);
        let settings = state.settings.clone();
        if runtime_paused || settings.as_ref().is_some_and(|value| value.paused) {
            return Err(BrokerError::Denied(PolicyReason::Paused));
        }
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        if settings.as_ref().is_some_and(|settings| {
            !settings_allows_data(runtime_paused, settings, &session.target.target)
        }) {
            return Err(BrokerError::Denied(PolicyReason::PolicyNever));
        }
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

        let target = session.target.target.clone();
        let signal = match payload.action {
            crate::protocol::ControlAction::Dismiss => {
                let visible = session.visible.take().expect("visible suggestion checked");
                let aggregate_day = visible.aggregate_day;
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
                aggregate_day.map(|day| (day, PersonalizationSignal::Dismissed))
            }
            crate::protocol::ControlAction::AcceptWord
            | crate::protocol::ControlAction::AcceptAll => {
                let visible = session.visible.take().expect("visible suggestion checked");
                let aggregate_day = visible.aggregate_day;
                let (acceptance, text) =
                    if payload.action == crate::protocol::ControlAction::AcceptWord {
                        let parts = accept_word(&visible.payload.text);
                        (Acceptance::Word, parts.accepted)
                    } else {
                        (Acceptance::All, visible.payload.text.clone())
                    };
                let commit = CommitPreparePayload {
                    fingerprint: visible.payload.fingerprint.clone(),
                    suggestion_id: visible.payload.suggestion_id.clone(),
                    text,
                    acceptance,
                };
                let suggestion_id = visible.payload.suggestion_id.clone();
                let expires_at = Instant::now() + self.inner.config.commit_result_lease;
                let pending = PendingCommit {
                    coordinates,
                    fingerprint: visible.payload.fingerprint,
                    suggestion_id: visible.payload.suggestion_id,
                    request_id: request_id.clone(),
                    expires_at,
                };
                if session
                    .sink
                    .send(BrokerEvent::CommitPrepare {
                        coordinates,
                        payload: commit,
                        request_id,
                    })
                    .is_err()
                {
                    self.inner.metrics.record_commit_failure();
                    return Err(BrokerError::EventSinkClosed);
                }
                session.pending = Some(pending);
                self.renew_context_authority_lease(session);
                self.inner.metrics.record_commit_prepared();
                let broker = self.clone();
                tokio::spawn(async move {
                    time::sleep(broker.inner.config.commit_result_lease).await;
                    broker
                        .expire_pending_commit(coordinates, suggestion_id, expires_at)
                        .await;
                });
                aggregate_day.map(|day| {
                    (
                        day,
                        if acceptance == Acceptance::Word {
                            PersonalizationSignal::AcceptedWord
                        } else {
                            PersonalizationSignal::AcceptedAll
                        },
                    )
                })
            }
            crate::protocol::ControlAction::Request
            | crate::protocol::ControlAction::Pause
            | crate::protocol::ControlAction::Resume
            | crate::protocol::ControlAction::PauseToggle => {
                return Err(BrokerError::InvalidPayload);
            }
        };
        let settings_revision = state.settings_revision;
        if let Some((event_day, signal)) = signal {
            let _ = self.queue_outcome(&target, settings_revision, event_day, signal);
        }
        drop(state);
        Ok(())
    }

    // Commit lease validation and state retirement are one atomic audit path.
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
        let pending = session
            .pending
            .as_ref()
            .ok_or(BrokerError::NoPendingCommit)?;
        if pending.coordinates != coordinates
            || pending.fingerprint != payload.fingerprint
            || pending.suggestion_id != payload.suggestion_id
        {
            self.inner.metrics.record_commit_failure();
            return Err(BrokerError::Stale);
        }
        if pending.expires_at <= Instant::now() {
            let pending = session.pending.take().expect("pending commit checked");
            send_pending_clear(session, pending, ReasonCode::Expired);
            self.inner.metrics.record_commit_failure();
            return Err(BrokerError::CommitLeaseExpired);
        }
        validate_commit_authority(&session.authority, payload.status)?;
        session.pending.take().expect("pending commit checked");

        match payload.status {
            CommitStatus::Applied => {
                // A commit mutates the document and retires every byte derived
                // from the old revision. The adapter must provide fresh context
                // and request a newly generated continuation; a cached suffix
                // cannot inherit authority or restart a relative display TTL.
                retire_applied_state(session);
                self.inner.metrics.record_commit_applied();
            }
            CommitStatus::DispatchedUnverified => {
                invalidate_context_authority_lease(session);
                session.context = None;
            }
            CommitStatus::Stale | CommitStatus::Blocked | CommitStatus::Failed => {
                // The adapter's terminal result says the pre-commit document
                // authority is no longer usable. Do not retain that context
                // for another request until the adapter supplies newer state.
                revoke_context_authority(session, &self.inner.metrics, ReasonCode::Stale);
                self.inner.metrics.record_commit_failure();
            }
        }
        drop(state);
        Ok(())
    }

    pub async fn set_paused(&self, paused: bool) -> bool {
        let mut state = self.inner.state.lock().await;
        let changed = transition_paused(&mut state, paused, &self.inner.metrics);
        let effective_paused = state_effective_paused(&state);
        let event = changed.then(|| authority_changed(&state));
        drop(state);
        if let Some(event) = event {
            let _ = self.inner.authority_events.send(event);
        }
        if effective_paused {
            self.flush_outcomes_before_pause_ack().await;
        }
        self.is_paused().await
    }

    pub async fn toggle_paused(&self) -> bool {
        let mut state = self.inner.state.lock().await;
        let paused = !state.paused;
        let _ = transition_paused(&mut state, paused, &self.inner.metrics);
        let effective_paused = state_effective_paused(&state);
        let event = authority_changed(&state);
        drop(state);
        let _ = self.inner.authority_events.send(event);
        if effective_paused {
            self.flush_outcomes_before_pause_ack().await;
        }
        self.is_paused().await
    }

    #[must_use]
    pub fn subscribe_authority_changes(&self) -> broadcast::Receiver<AuthorityChangedPayload> {
        self.inner.authority_events.subscribe()
    }

    pub async fn register_policy_client(&self, connection_id: String) {
        let mut state = self.inner.state.lock().await;
        state.policy_clients.insert(connection_id, None);
    }

    pub async fn unregister_policy_client(&self, connection_id: &str) {
        self.inner
            .state
            .lock()
            .await
            .policy_clients
            .remove(connection_id);
    }

    pub async fn acknowledge_authority(
        &self,
        connection_id: &str,
        authority_epoch: u64,
    ) -> Result<(), BrokerError> {
        let mut state = self.inner.state.lock().await;
        if authority_epoch > state.authority_epoch {
            return Err(BrokerError::InvalidPayload);
        }
        let current_epoch = state.authority_epoch;
        let acknowledged = state
            .policy_clients
            .get_mut(connection_id)
            .ok_or(BrokerError::InvalidCapability)?;
        let acknowledged_epoch = authority_epoch.min(current_epoch);
        *acknowledged =
            Some(acknowledged.map_or(acknowledged_epoch, |value| value.max(acknowledged_epoch)));
        Ok(())
    }

    pub async fn authority_snapshot(&self) -> AuthoritySnapshot {
        let state = self.inner.state.lock().await;
        AuthoritySnapshot {
            authority_epoch: state.authority_epoch,
            settings_revision: state.settings_revision,
            paused: state_effective_paused(&state),
            control_plane_degraded: state.control_plane_condition.is_degraded()
                || state.control_plane_mutation_in_progress,
            pending_acknowledgements: state
                .policy_clients
                .values()
                .filter(|acknowledged| **acknowledged != Some(state.authority_epoch))
                .count(),
        }
    }

    pub async fn resolve_policy(&self, target: &TargetDescriptor) -> PolicyStatusPayload {
        let state = self.inner.state.lock().await;
        if let Some(settings) = state.settings.as_ref() {
            policy_status(
                runtime_control_paused(&state),
                state.authority_epoch,
                settings,
                target,
            )
        } else {
            denied_policy_status(
                state.authority_epoch,
                state.settings_revision,
                runtime_control_paused(&state),
                PolicyResolutionReason::UnknownIdentity,
            )
        }
    }

    pub async fn control_plane_snapshot(&self) -> Result<ControlPlaneSnapshot, BrokerError> {
        self.inner
            .outcome_recorder
            .clone()
            .ok_or(BrokerError::ControlPlaneUnavailable)?
            .snapshot()
            .await
    }

    #[must_use]
    pub fn outcome_recorder_health(&self) -> OutcomeRecorderHealth {
        self.inner.outcome_recorder.as_ref().map_or(
            OutcomeRecorderHealth {
                available: false,
                dropped_signals: 0,
                write_failures: 0,
            },
            OutcomeRecorder::health,
        )
    }

    pub async fn replace_settings(
        &self,
        expected_revision: u64,
        next: SettingsV1,
    ) -> Result<ControlPlaneSnapshot, BrokerError> {
        let control_plane = self
            .inner
            .control_plane
            .clone()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        // Serialize the complete mutation, not merely the on-disk CAS. The
        // broker must never reinstall an older snapshot after a newer commit.
        let _mutation = self.inner.control_plane_mutation.lock().await;
        self.begin_control_plane_mutation().await;
        let committed_settings = next.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            match control_plane.replace_settings(expected_revision, next) {
                Ok(_) => match control_plane.snapshot() {
                    Ok(snapshot) => SettingsReplaceOutcome::Updated(snapshot),
                    Err(error) => SettingsReplaceOutcome::CommittedDegraded {
                        settings: committed_settings,
                        error,
                    },
                },
                Err(error @ ControlPlaneError::SettingsCommittedReconciliation { .. }) => {
                    SettingsReplaceOutcome::CommittedDegraded {
                        settings: committed_settings,
                        error,
                    }
                }
                Err(
                    error @ ControlPlaneError::Settings(SettingsStoreError::CommitStateUnknown),
                ) => SettingsReplaceOutcome::CommitUnknown(error),
                Err(
                    error @ ControlPlaneError::Personalization(PersonalizationStoreError::Storage(
                        PrivateStorageError::CommitStateUnknown,
                    )),
                ) => SettingsReplaceOutcome::RejectedDegraded(error),
                Err(error) => SettingsReplaceOutcome::Rejected(error),
            }
        })
        .await;
        let Ok(outcome) = outcome else {
            // The blocking task may have panicked after the settings file
            // committed. Its commit state is unknowable, so do not reopen
            // the data plane until a later coherent control-plane repair.
            self.fail_control_plane_mutation_unknown().await;
            return Err(BrokerError::ControlPlaneTask);
        };

        match outcome {
            SettingsReplaceOutcome::Updated(updated) => {
                self.install_settings_authority(updated.settings.clone(), false)
                    .await;
                Ok(updated)
            }
            SettingsReplaceOutcome::CommittedDegraded { settings, error } => {
                // The settings file is already authoritative. If retention
                // reconciliation or the post-commit snapshot failed, revoke
                // every live lease and force the data plane paused until a
                // successful control-plane operation proves coherence again.
                self.install_settings_authority(settings, true).await;
                Err(BrokerError::SettingsCommittedDegraded(error))
            }
            SettingsReplaceOutcome::CommitUnknown(error) => {
                self.fail_control_plane_mutation_unknown().await;
                Err(BrokerError::SettingsCommitUnknown(error))
            }
            SettingsReplaceOutcome::RejectedDegraded(error) => {
                self.fail_control_plane_mutation_recoverable().await;
                Err(BrokerError::ControlPlane(error))
            }
            SettingsReplaceOutcome::Rejected(error) => {
                self.reject_control_plane_mutation().await;
                Err(BrokerError::ControlPlane(error))
            }
        }
    }

    pub async fn clear_personalization(&self) -> Result<(bool, ControlPlaneSnapshot), BrokerError> {
        let _mutation = self.inner.control_plane_mutation.lock().await;
        let recorder = self
            .inner
            .outcome_recorder
            .clone()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        // Hold the broker state lock across the recorder's FIFO clear barrier.
        // Every pre-clear outcome is therefore processed before the clear, no
        // post-clear outcome can be queued early, and visible suggestions lose
        // their link to a Shown aggregate that no longer exists.
        let mut state = self.inner.state.lock().await;
        for session in state.sessions.values_mut() {
            if let Some(visible) = session.visible.as_mut() {
                visible.aggregate_day = None;
            }
        }
        let result = recorder.clear().await?;
        let recovered = state.control_plane_condition == ControlPlaneCondition::Recoverable;
        if recovered {
            state.control_plane_condition = ControlPlaneCondition::Healthy;
            state.settings_revision = result.1.settings.revision;
            state.settings = Some(result.1.settings.clone());
            state.authority_epoch = state
                .authority_epoch
                .saturating_add(1)
                .min(MAX_SAFE_COUNTER);
        }
        let event = recovered.then(|| authority_changed(&state));
        drop(state);
        if let Some(event) = event {
            let _ = self.inner.authority_events.send(event);
        }
        Ok(result)
    }

    async fn install_settings_authority(&self, settings: SettingsV1, degraded: bool) {
        let mut state = self.inner.state.lock().await;
        for session in state.sessions.values_mut() {
            revoke_context_authority(session, &self.inner.metrics, ReasonCode::PolicyNever);
        }
        state.sessions.clear();
        state.control_plane_mutation_in_progress = false;
        state.control_plane_condition = if degraded {
            ControlPlaneCondition::Recoverable
        } else {
            ControlPlaneCondition::Healthy
        };
        state.settings_revision = settings.revision;
        state.settings = Some(settings);
        state.authority_epoch = state
            .authority_epoch
            .saturating_add(1)
            .min(MAX_SAFE_COUNTER);
        let event = authority_changed(&state);
        drop(state);
        let _ = self.inner.authority_events.send(event);
    }

    async fn begin_control_plane_mutation(&self) {
        let mut state = self.inner.state.lock().await;
        for session in state.sessions.values_mut() {
            revoke_context_authority(session, &self.inner.metrics, ReasonCode::PolicyNever);
        }
        state.sessions.clear();
        state.control_plane_mutation_in_progress = true;
        state.authority_epoch = state
            .authority_epoch
            .saturating_add(1)
            .min(MAX_SAFE_COUNTER);
        let event = authority_changed(&state);
        drop(state);
        let _ = self.inner.authority_events.send(event);
    }

    async fn reject_control_plane_mutation(&self) {
        let mut state = self.inner.state.lock().await;
        state.control_plane_mutation_in_progress = false;
        state.authority_epoch = state
            .authority_epoch
            .saturating_add(1)
            .min(MAX_SAFE_COUNTER);
        let event = authority_changed(&state);
        drop(state);
        let _ = self.inner.authority_events.send(event);
    }

    async fn fail_control_plane_mutation_unknown(&self) {
        let mut state = self.inner.state.lock().await;
        state.control_plane_mutation_in_progress = false;
        state.control_plane_condition = ControlPlaneCondition::RestartRequired;
        state.authority_epoch = state
            .authority_epoch
            .saturating_add(1)
            .min(MAX_SAFE_COUNTER);
        let event = authority_changed(&state);
        drop(state);
        let _ = self.inner.authority_events.send(event);
    }

    async fn fail_control_plane_mutation_recoverable(&self) {
        let mut state = self.inner.state.lock().await;
        state.control_plane_mutation_in_progress = false;
        if state.control_plane_condition == ControlPlaneCondition::Healthy {
            state.control_plane_condition = ControlPlaneCondition::Recoverable;
        }
        // Never downgrade an earlier commit-unknown state. Only a coherent
        // settings installation (or process restart/reload) may clear it.
        state.authority_epoch = state
            .authority_epoch
            .saturating_add(1)
            .min(MAX_SAFE_COUNTER);
        let event = authority_changed(&state);
        drop(state);
        let _ = self.inner.authority_events.send(event);
    }

    async fn flush_outcomes_before_pause_ack(&self) {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return;
        };
        if let Err(error) = recorder.flush().await {
            // A disconnected recorder cannot perform later writes. Keep the
            // pause effective and report the operational loss without logging
            // any context or suggestion content.
            eprintln!("badi-broker: outcome recorder pause fence failed: {error}");
        }
    }

    fn queue_outcome(
        &self,
        target: &TargetDescriptor,
        expected_settings_revision: u64,
        event_day: u64,
        signal: PersonalizationSignal,
    ) -> bool {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return false;
        };
        let Ok(identity) = StableIdentity::from_target(target) else {
            return false;
        };
        recorder.try_signal(OutcomeCommand::Signal {
            expected_settings_revision,
            event_day,
            identity,
            provider: PersonalizationProvider::from(self.inner.provider_kind),
            signal,
        })
    }

    pub async fn is_paused(&self) -> bool {
        let state = self.inner.state.lock().await;
        state_effective_paused(&state)
    }

    pub async fn session_count(&self) -> u64 {
        u64::try_from(self.inner.state.lock().await.sessions.len()).unwrap_or(u64::MAX)
    }

    pub async fn active_locator(&self) -> Option<ActiveLocator> {
        let state = self.inner.state.lock().await;
        active_locator_from_state(&state)
    }

    pub async fn health_snapshot(&self) -> HealthSnapshot {
        let state = self.inner.state.lock().await;
        HealthSnapshot {
            provider: self.provider_kind(),
            paused: state_effective_paused(&state),
            authority_epoch: state.authority_epoch,
            settings_revision: state.settings_revision,
            control_plane_degraded: state.control_plane_condition.is_degraded(),
            sessions: u64::try_from(state.sessions.len()).unwrap_or(u64::MAX),
            max_frame_bytes: MAX_FRAME_BYTES,
            metrics: self.inner.metrics.snapshot(),
            active: active_locator_from_state(&state),
        }
    }
}

fn current_unix_day() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs() / 86_400)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub provider: ProviderKind,
    pub paused: bool,
    pub authority_epoch: u64,
    pub settings_revision: u64,
    pub control_plane_degraded: bool,
    pub sessions: u64,
    pub max_frame_bytes: usize,
    pub metrics: MetricsSnapshot,
    pub active: Option<ActiveLocator>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthoritySnapshot {
    pub authority_epoch: u64,
    pub settings_revision: u64,
    pub paused: bool,
    pub control_plane_degraded: bool,
    pub pending_acknowledgements: usize,
}

fn invalidate_context_authority_lease(session: &mut SessionState) {
    session.context_lease_generation = session.context_lease_generation.wrapping_add(1);
    if let Some(cancellation) = session.context_lease_cancellation.take() {
        cancellation.cancel();
    }
}

fn revoke_context_authority(session: &mut SessionState, metrics: &Metrics, reason: ReasonCode) {
    invalidate_context_authority_lease(session);
    retire_session(session, metrics, reason);
    session.context = None;
}

fn active_locator_from_state(state: &BrokerState) -> Option<ActiveLocator> {
    if state_effective_paused(state) {
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

fn retire_applied_state(session: &mut SessionState) {
    invalidate_context_authority_lease(session);
    session.generation = session.generation.wrapping_add(1);
    if let Some(cancellation) = session.cancellation.take() {
        cancellation.cancel();
    }
    session.context = None;
    session.visible = None;
}

fn transition_paused(state: &mut BrokerState, paused: bool, metrics: &Metrics) -> bool {
    if state.paused == paused {
        return false;
    }
    state.paused = paused;
    state.authority_epoch = state
        .authority_epoch
        .saturating_add(1)
        .min(MAX_SAFE_COUNTER);
    if state_effective_paused(state) {
        for session in state.sessions.values_mut() {
            revoke_context_authority(session, metrics, ReasonCode::Paused);
        }
    }
    true
}

fn authority_changed(state: &BrokerState) -> AuthorityChangedPayload {
    AuthorityChangedPayload {
        authority_epoch: state.authority_epoch,
        settings_revision: state.settings_revision,
        paused: state_effective_paused(state),
    }
}

fn state_effective_paused(state: &BrokerState) -> bool {
    runtime_control_paused(state)
        || state
            .settings
            .as_ref()
            .is_some_and(|settings| settings.paused)
}

fn runtime_control_paused(state: &BrokerState) -> bool {
    state.paused
        || state.control_plane_condition.is_degraded()
        || state.control_plane_mutation_in_progress
}

fn settings_allows_data(
    runtime_paused: bool,
    settings: &SettingsV1,
    target: &TargetDescriptor,
) -> bool {
    let resolution = settings.resolve_target_validated(target);
    !runtime_paused
        && resolution.configured
        && resolution.allows_context_read()
        && resolution.allows_display()
        && resolution.allows_suggestion()
}

fn policy_status(
    runtime_paused: bool,
    authority_epoch: u64,
    settings: &SettingsV1,
    target: &TargetDescriptor,
) -> PolicyStatusPayload {
    let resolution = settings.resolve_target_validated(target);
    let paused = runtime_paused || resolution.paused;
    if paused {
        return denied_policy_status(
            authority_epoch,
            settings.revision,
            true,
            PolicyResolutionReason::GlobalDisabled,
        );
    }
    if !resolution.identity_known {
        return denied_policy_status(
            authority_epoch,
            settings.revision,
            false,
            PolicyResolutionReason::UnknownIdentity,
        );
    }
    if !resolution.configured {
        return denied_policy_status(
            authority_epoch,
            settings.revision,
            false,
            PolicyResolutionReason::DefaultPolicy,
        );
    }
    if !resolution.allows_context_read() {
        return denied_policy_status(
            authority_epoch,
            settings.revision,
            false,
            PolicyResolutionReason::ContextDisabled,
        );
    }
    if !resolution.allows_display() || !resolution.allows_suggestion() {
        return denied_policy_status(
            authority_epoch,
            settings.revision,
            false,
            PolicyResolutionReason::SuggestionsDisabled,
        );
    }
    PolicyStatusPayload {
        authority_epoch,
        settings_revision: settings.revision,
        paused: false,
        activation: Activation::Always,
        context_allowed: true,
        display_allowed: true,
        suggestions_allowed: true,
        learning_allowed: resolution.allows_learning(),
        reason: PolicyResolutionReason::MatchedRule,
    }
}

fn denied_policy_status(
    authority_epoch: u64,
    settings_revision: u64,
    paused: bool,
    reason: PolicyResolutionReason,
) -> PolicyStatusPayload {
    PolicyStatusPayload {
        authority_epoch,
        settings_revision,
        paused,
        activation: Activation::Never,
        context_allowed: false,
        display_allowed: false,
        suggestions_allowed: false,
        learning_allowed: false,
        reason,
    }
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
    #[error("control_plane")]
    ControlPlane(#[from] ControlPlaneError),
    #[error("control_plane_task")]
    ControlPlaneTask,
    #[error("control_plane_unavailable")]
    ControlPlaneUnavailable,
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
    #[error("outcome_recorder_thread")]
    OutcomeRecorderThread(#[from] std::io::Error),
    #[error("protocol")]
    Protocol(#[from] crate::protocol::ProtocolError),
    #[error("provider_busy")]
    ProviderBusy,
    #[error("session_already_open")]
    SessionAlreadyOpen,
    #[error("shutting_down")]
    ShuttingDown,
    #[error("settings_committed_degraded")]
    SettingsCommittedDegraded(#[source] ControlPlaneError),
    #[error("settings_commit_unknown")]
    SettingsCommitUnknown(#[source] ControlPlaneError),
    #[error("stale")]
    Stale,
    #[error("unknown_session")]
    UnknownSession,
}

#[cfg(test)]
mod tests {
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
        validate_commit_authority,
    };
    use crate::control_plane::ControlPlane;
    use crate::personalization::PersonalizationSignal;
    use crate::protocol::{
        Activation, AdapterKind, Capability, CommitResultPayload, CommitStatus,
        ContextChangedPayload, ControlAction, Coordinates, FieldDescriptor, FieldPurpose,
        OffsetUnit, Origin, OriginScheme, ProviderKind, ReasonCode, Selection,
        SessionControlRequestPayload, SessionId, SessionOpenPayload, SuggestRequestPayload,
        TargetDescriptor, TargetKind,
    };
    use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
    use crate::settings::{
        BrowserAdapter, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA, SettingsV1,
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

    async fn setup(
        provider: std::sync::Arc<dyn CompletionProvider>,
        config: BrokerConfig,
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
                    adapter_kind: AdapterKind::Browser,
                    capabilities: vec![
                        Capability::Context,
                        Capability::Suggestion,
                        Capability::CommitApplied,
                    ],
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

    fn controlled_settings(revision: u64, allowed: bool, learn: bool) -> SettingsV1 {
        let decision = if allowed {
            PermissionDecision::Allow
        } else {
            PermissionDecision::Block
        };
        SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision,
            paused: false,
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

    fn controlled_learning_settings(revision: u64, retention: RetentionPermission) -> SettingsV1 {
        SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision,
            paused: false,
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
            let (update, suggestion_id) = prepare_commit_result_case(
                &broker,
                session_id,
                &mut events,
                ControlAction::AcceptAll,
            )
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
        let paths = StoragePaths::new(temporary.path().join("config/badi"), &data_dir)
            .expect("storage paths");
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
}
