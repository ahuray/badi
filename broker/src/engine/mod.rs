//! The session engine behind the broker socket. One state lock orders every
//! session, pause and settings-authority decision; each submodule adds the
//! `Broker` operations of one concern.

mod acceptance;
mod authority;
mod error;
mod events;
mod generation;
mod outcomes;
mod probe;
mod session;
mod settings;
mod status;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, MutexGuard, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

use crate::control_plane::ControlPlane;
use crate::metrics::Metrics;
use crate::policy::PolicyReason;
use crate::protocol::{
    AdapterKind, AuthorityChangedPayload, Capability, ContextChangedPayload, Coordinates,
    DEFAULT_SUGGESTION_TTL_MS, MAX_SAFE_COUNTER, ProviderKind, ReasonCode, SessionId,
    SessionOpenPayload, SuggestionShowPayload,
};
use crate::provider::{CompletionProvider, RequestTrigger};
use crate::settings::SettingsV2;

pub use authority::AuthoritySnapshot;
pub use error::BrokerError;
pub use events::{BrokerEvent, BrokerEventSink};
pub use outcomes::OutcomeRecorderHealth;
pub use status::HealthSnapshot;

use outcomes::OutcomeRecorder;

/// Default broker-local time allowed for an adapter to report a commit result.
pub const DEFAULT_COMMIT_RESULT_LEASE_MS: u64 = 1_500;
/// Hard ceiling for the broker-local commit-result lease.
pub const MAX_COMMIT_RESULT_LEASE_MS: u64 = 5_000;
/// Default silence window before retained context authority is revoked.
pub const DEFAULT_CONTEXT_AUTHORITY_LEASE_MS: u64 = 3_000;
const MIN_CONTEXT_AUTHORITY_LEASE_MS: u64 = 500;
const MAX_CONTEXT_AUTHORITY_LEASE_MS: u64 = 10_000;
/// Default number of provider generations allowed across the entire broker.
pub const DEFAULT_PROVIDER_CONCURRENCY: usize = 4;
/// Hard upper bound even when a caller supplies a larger configuration value.
pub const MAX_PROVIDER_CONCURRENCY: usize = 16;
/// Maximum receiver-local age from an accepted request to provider completion.
pub const MAX_GENERATION_TIMEOUT_MS: u64 = 600;
/// The same limit for an explicit request, which has a longer writing budget.
pub const MAX_EXPLICIT_GENERATION_TIMEOUT_MS: u64 = 1_250;
/// Native panels need enough time to read and accept a continuation.
pub const NATIVE_SUGGESTION_TTL_MS: u64 = 5_000;
const MAX_CONFIGURED_SUGGESTION_TTL_MS: u64 = 600;

#[derive(Clone, Copy, Debug)]
pub struct BrokerConfig {
    pub debounce: Duration,
    pub provider_timeout: Duration,
    /// Includes broker debounce; late output is never displayed with a fresh TTL.
    pub generation_timeout: Duration,
    /// `generation_timeout` for explicit requests; never shorter than it.
    pub explicit_generation_timeout: Duration,
    /// Maximum provider generations admitted across all broker connections.
    pub provider_concurrency: usize,
    pub suggestion_ttl: Duration,
    /// Receiver-local silence lease for retained context and derived authority.
    pub context_authority_lease: Duration,
    /// Receiver-local lease; adapters must report the authorized commit before it expires.
    pub commit_result_lease: Duration,
}

impl BrokerConfig {
    const fn generation_timeout_for(&self, trigger: RequestTrigger) -> Duration {
        match trigger {
            RequestTrigger::Automatic => self.generation_timeout,
            RequestTrigger::Explicit => self.explicit_generation_timeout,
        }
    }

    fn suggestion_ttl_for(&self, authority: &SessionAuthority) -> Duration {
        if authority.has_reading_window() {
            Duration::from_millis(NATIVE_SUGGESTION_TTL_MS)
        } else {
            self.suggestion_ttl
        }
    }

    /// `generation_timeout` is the limit of the request this lease admits, so
    /// a suggestion shown at that limit keeps its full reading window.
    fn context_authority_lease_for(
        &self,
        authority: &SessionAuthority,
        generation_timeout: Duration,
    ) -> Duration {
        if authority.has_reading_window() {
            self.context_authority_lease
                .max(Duration::from_millis(NATIVE_SUGGESTION_TTL_MS) + generation_timeout)
        } else {
            self.context_authority_lease
        }
    }

    /// Holds every limit within its hard bounds, whatever the caller supplied.
    fn clamped(self) -> Self {
        let millis = Duration::from_millis;
        let generation_timeout = self
            .generation_timeout
            .clamp(millis(1), millis(MAX_GENERATION_TIMEOUT_MS));
        Self {
            provider_concurrency: self.provider_concurrency.clamp(1, MAX_PROVIDER_CONCURRENCY),
            suggestion_ttl: self
                .suggestion_ttl
                .clamp(millis(1), millis(MAX_CONFIGURED_SUGGESTION_TTL_MS)),
            generation_timeout,
            explicit_generation_timeout: self.explicit_generation_timeout.clamp(
                generation_timeout,
                millis(MAX_EXPLICIT_GENERATION_TIMEOUT_MS),
            ),
            commit_result_lease: self
                .commit_result_lease
                .clamp(millis(1), millis(MAX_COMMIT_RESULT_LEASE_MS)),
            context_authority_lease: self.context_authority_lease.clamp(
                millis(MIN_CONTEXT_AUTHORITY_LEASE_MS),
                millis(MAX_CONTEXT_AUTHORITY_LEASE_MS),
            ),
            ..self
        }
    }
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            // Adapters own user-idle acquisition timing. A second production
            // debounce here only consumes the single input-to-visible budget.
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_millis(1_300),
            generation_timeout: Duration::from_millis(MAX_GENERATION_TIMEOUT_MS),
            explicit_generation_timeout: Duration::from_millis(MAX_EXPLICIT_GENERATION_TIMEOUT_MS),
            provider_concurrency: DEFAULT_PROVIDER_CONCURRENCY,
            suggestion_ttl: Duration::from_millis(DEFAULT_SUGGESTION_TTL_MS),
            context_authority_lease: Duration::from_millis(DEFAULT_CONTEXT_AUTHORITY_LEASE_MS),
            commit_result_lease: Duration::from_millis(DEFAULT_COMMIT_RESULT_LEASE_MS),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionAuthority {
    pub protocol_version: u8,
    pub adapter_kind: AdapterKind,
    pub capabilities: Vec<Capability>,
}

impl SessionAuthority {
    fn grants(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Protocol v2 native panels and editors give the user a human reading
    /// window; protocol v1 keeps its configured lease. Revision and focus
    /// guards still revoke both.
    fn has_reading_window(&self) -> bool {
        self.protocol_version == 2
            && matches!(
                self.adapter_kind,
                AdapterKind::Fcitx | AdapterKind::Obsidian | AdapterKind::Terminal
            )
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
    settings: Option<SettingsV2>,
    /// The latest authority epoch each policy connection acknowledged.
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
    context: Option<ContextChangedPayload>,
    context_seen_at_coordinates: bool,
    context_lease_generation: u64,
    context_lease_cancellation: Option<CancellationToken>,
    generation: u64,
    cancellation: Option<CancellationToken>,
    visible: Option<VisibleSuggestion>,
    pending: Option<PendingCommit>,
    sink: BrokerEventSink,
}

struct VisibleSuggestion {
    payload: SuggestionShowPayload,
    expires_at: Instant,
    request_id: Option<String>,
    /// The day of the recorded `Shown` aggregate this suggestion belongs to.
    aggregate_day: Option<u64>,
}

struct PendingCommit {
    coordinates: Coordinates,
    fingerprint: String,
    suggestion_id: String,
    request_id: Option<String>,
    expires_at: Instant,
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
        settings: Option<SettingsV2>,
        outcome_recorder: Option<OutcomeRecorder>,
    ) -> Self {
        let config = config.clamped();
        let (authority_events, _) = broadcast::channel(32);
        Self {
            inner: Arc::new(BrokerInner {
                control_plane,
                control_plane_mutation: Mutex::new(()),
                provider_kind: provider.kind(),
                provider,
                provider_admissions: Arc::new(Semaphore::new(config.provider_concurrency)),
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

    /// Completes once the provider can no longer serve requests.
    pub async fn provider_exited(&self) {
        self.inner.provider.exited().await;
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

    /// Releases the state lock, then announces its authority epoch to every
    /// policy connection.
    fn publish_authority_change(&self, state: MutexGuard<'_, BrokerState>) {
        let event = state.authority_changed();
        drop(state);
        let _ = self.inner.authority_events.send(event);
    }
}

impl BrokerState {
    /// Paused by the user, or while the control plane is degraded or mid-mutation.
    fn runtime_paused(&self) -> bool {
        self.paused
            || self.control_plane_condition.is_degraded()
            || self.control_plane_mutation_in_progress
    }

    fn effective_paused(&self) -> bool {
        self.runtime_paused()
            || self
                .settings
                .as_ref()
                .is_some_and(|settings| settings.paused)
    }

    fn advance_authority_epoch(&mut self) {
        self.authority_epoch = self.authority_epoch.saturating_add(1).min(MAX_SAFE_COUNTER);
    }

    fn authority_changed(&self) -> AuthorityChangedPayload {
        AuthorityChangedPayload {
            authority_epoch: self.authority_epoch,
            settings_revision: self.settings_revision,
            paused: self.effective_paused(),
        }
    }

    /// The session, while current settings still let its target read context
    /// and receive suggestions.
    fn session_with_data_access(
        &mut self,
        session_id: SessionId,
    ) -> Result<&mut SessionState, BrokerError> {
        let runtime_paused = self.runtime_paused();
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(BrokerError::UnknownSession)?;
        if self.settings.as_ref().is_some_and(|settings| {
            !authority::settings_allow_data(runtime_paused, settings, &session.target.target)
        }) {
            return Err(BrokerError::Denied(PolicyReason::PolicyNever));
        }
        Ok(session)
    }

    fn close_all_sessions(&mut self, metrics: &Metrics, reason: ReasonCode) {
        for session in self.sessions.values_mut() {
            session.revoke_context_authority(metrics, reason);
        }
        self.sessions.clear();
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
