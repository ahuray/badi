//! The session engine behind the broker socket. One state lock orders every
//! session, pause and settings-authority decision; each submodule adds the
//! `Broker` operations of one concern.

mod acceptance;
mod authority;
mod desktop;
mod editor;
mod error;
mod events;
mod generation;
mod outcomes;
mod probe;
mod revision;
mod session;
mod settings;
mod snooze;
mod status;
#[cfg(test)]
mod tests;
mod type_through;

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, MutexGuard, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

use crate::control_plane::ControlPlane;
use crate::metrics::Metrics;
use crate::policy::PolicyReason;
use crate::protocol::{
    AdapterKind, AuthorityChangedPayload, Capability, ContextChangedPayload, Coordinates,
    DEFAULT_SUGGESTION_TTL_MS, DesktopStatePayload, MAX_SAFE_COUNTER, ProviderKind, ReasonCode,
    SessionId, SessionOpenPayload, SuggestionShowPayload,
};
use crate::provider::{CompletionProvider, RequestTrigger};
use crate::settings::{SettingsV2, StableIdentity};

pub use authority::AuthoritySnapshot;
pub use desktop::{QUIET_START_DELAY, SCREEN_SHARE_GRACE};
pub use error::BrokerError;
pub use events::{BrokerEvent, BrokerEventSink};
pub use outcomes::OutcomeRecorderHealth;
pub use revision::{REVISION_TIMEOUT, REVISION_TTL};
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
/// The limit for a spelling-only field's check, beyond its writing budget.
pub const SPELLING_GENERATION_TIMEOUT_MS: u64 = 2_750;
/// Native panels need enough time to read and accept a continuation.
pub const NATIVE_SUGGESTION_TTL_MS: u64 = 5_000;
/// Opt-in reading window; older v2 clients retain their five-second ceiling.
pub const LONG_READING_WINDOW_MS: u64 = 15_000;
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
    /// How long a screen share stays held after the helper that reported it
    /// disconnects, unless a helper reports again sooner.
    pub screen_share_grace: Duration,
}

impl BrokerConfig {
    /// A spelling-only check's provider call has its whole generation limit.
    const fn provider_timeout_for(&self, trigger: RequestTrigger) -> Duration {
        match trigger {
            RequestTrigger::Spelling => Duration::from_millis(SPELLING_GENERATION_TIMEOUT_MS),
            RequestTrigger::Automatic | RequestTrigger::Explicit => self.provider_timeout,
        }
    }

    const fn generation_timeout_for(&self, trigger: RequestTrigger) -> Duration {
        match trigger {
            RequestTrigger::Automatic => self.generation_timeout,
            RequestTrigger::Explicit => self.explicit_generation_timeout,
            RequestTrigger::Spelling => Duration::from_millis(SPELLING_GENERATION_TIMEOUT_MS),
        }
    }

    fn suggestion_ttl_for(&self, authority: &SessionAuthority) -> Duration {
        if authority.has_reading_window() {
            Duration::from_millis(if authority.grants(Capability::LongReadingWindow) {
                LONG_READING_WINDOW_MS
            } else {
                NATIVE_SUGGESTION_TTL_MS
            })
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
                .max(self.suggestion_ttl_for(authority) + generation_timeout)
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
            screen_share_grace: SCREEN_SHARE_GRACE,
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
                AdapterKind::Fcitx
                    | AdapterKind::Obsidian
                    | AdapterKind::Terminal
                    | AdapterKind::Neovim
                    | AdapterKind::Libreoffice
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
    /// The model behind explicit revisions (ADR 0012), when there is one.
    reviser: std::sync::OnceLock<Arc<dyn crate::revision::service::Reviser>>,
    /// One revision runs at a time.
    revision_admissions: Arc<Semaphore>,
    config: BrokerConfig,
    metrics: Arc<Metrics>,
    shutdown: CancellationToken,
    authority_events: broadcast::Sender<AuthorityChangedPayload>,
    outcome_recorder: Option<OutcomeRecorder>,
    started: Instant,
    state: Mutex<BrokerState>,
    socket_path: std::sync::Mutex<Option<PathBuf>>,
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
    /// The last shown continuation that typing may carry (ADR 0004).
    type_through: Option<type_through::TypeThrough>,
    /// Where the caret last stood at a word start in front of another word,
    /// in the app or site of that context. Like a carry it outlives the
    /// session an observed field reopens on each edit, and it grants nothing.
    word_start: Option<(type_through::CarryScope, WordStart)>,
    /// A denial only: a freshly authorized observed session may inherit this
    /// field visit's dismissal, never context or a suggestion/commit grant.
    rebound_quiet: Option<ReboundQuiet>,
    /// Each connected desktop helper's latest report, by connection.
    desktop: HashMap<String, DesktopStatePayload>,
    /// Until when a share stays held after its helper disconnected mid-share;
    /// any new report ends it, and only its own timer clears it otherwise.
    share_grace: Option<Instant>,
    /// Editor connections that hold their terminal's focus.
    editor_focus: HashSet<String>,
    /// Apps and sites the user snoozed, until when (runtime only).
    snoozes: HashMap<StableIdentity, Instant>,
    /// Recent field visits, by target, with the words already counted toward
    /// learned words there, so each word counts once per field (ADR 0010).
    /// An observed field reopens its session as it changes; its target id
    /// stays for the visit.
    learned_visits: std::collections::VecDeque<(String, std::collections::BTreeSet<String>)>,
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
    /// An explicit revision of the selection, running or shown (ADR 0012).
    revision: Option<revision::SessionRevision>,
    escape_quiet: Option<EscapeQuiet>,
    sink: BrokerEventSink,
}

/// A caret that stood at the start of a word in front of another word
/// (`the |report`). While only letters are typed there and the text after
/// the caret stays the same, those letters are a new word, not the start of
/// the next one (`the fin|report`).
struct WordStart {
    before: String,
    after: String,
    /// Field text is not kept longer than typing a word takes.
    expires_at: Instant,
}

/// How long a word start lasts without a new context at it.
const WORD_START_LIFETIME: Duration = Duration::from_secs(30);

impl BrokerState {
    /// Whether `context`, in `scope`, holds letters typed at the last word
    /// start in front of another word.
    fn typed_at_word_start(
        &self,
        scope: &type_through::CarryScope,
        context: &ContextChangedPayload,
    ) -> bool {
        self.word_start
            .as_ref()
            .is_some_and(|(start_scope, start)| start_scope == scope && start.typed_in(context))
    }
}

impl WordStart {
    /// The word start that still holds for `context`: the caret is at one
    /// now, or only letters were typed at the earlier one since.
    fn follow(previous: Option<Self>, context: &ContextChangedPayload) -> Option<Self> {
        let at_start = context
            .before
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace)
            && context
                .after
                .chars()
                .next()
                .is_some_and(char::is_alphabetic);
        if at_start {
            return Some(Self {
                before: context.before.clone(),
                after: context.after.clone(),
                expires_at: Instant::now() + WORD_START_LIFETIME,
            });
        }
        previous.filter(|start| {
            Instant::now() < start.expires_at
                && start.after == context.after
                && context
                    .before
                    .strip_prefix(start.before.as_str())
                    .is_some_and(|typed| {
                        !typed.is_empty() && typed.chars().all(char::is_alphabetic)
                    })
        })
    }

    /// Whether `context` is this word start with letters typed at it.
    fn typed_in(&self, context: &ContextChangedPayload) -> bool {
        Instant::now() < self.expires_at
            && self.after == context.after
            && context.before.len() > self.before.len()
            && context.before.starts_with(&self.before)
    }
}

/// A dismissal's timed quiet for automatic suggestions in one field visit.
/// Bound to the focus epoch that was dismissed; an explicit request or a
/// new focus epoch ends it early.
#[derive(Clone, Copy)]
struct EscapeQuiet {
    focus_epoch: u64,
    until: Instant,
}

struct ReboundQuiet {
    target: crate::protocol::TargetDescriptor,
    until: Instant,
}

impl EscapeQuiet {
    fn active(self, focus_epoch: u64, now: Instant) -> bool {
        self.focus_epoch == focus_epoch && now < self.until
    }
}

struct VisibleSuggestion {
    payload: SuggestionShowPayload,
    /// Immutable shorter versions of this growing continuation. They share
    /// this generation and its original expiry, never another field/revision.
    earlier: Vec<SuggestionShowPayload>,
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
    /// Content-free estimate; counted only after a valid successful result.
    saved_words: u64,
    /// A revision's grant: no saved words, statistics or style sample.
    revision: bool,
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
        let mut state = BrokerState::default();
        if let Some(settings) = settings {
            state.install_settings(settings, &*provider);
        }
        Self {
            inner: Arc::new(BrokerInner {
                control_plane,
                control_plane_mutation: Mutex::new(()),
                provider_kind: provider.kind(),
                provider,
                provider_admissions: Arc::new(Semaphore::new(config.provider_concurrency)),
                reviser: std::sync::OnceLock::new(),
                revision_admissions: Arc::new(Semaphore::new(1)),
                config,
                metrics: Arc::new(Metrics::default()),
                shutdown: CancellationToken::new(),
                authority_events,
                outcome_recorder,
                started: Instant::now(),
                socket_path: std::sync::Mutex::new(None),
                state: Mutex::new(state),
            }),
        }
    }

    #[must_use]
    pub fn provider_kind(&self) -> ProviderKind {
        self.inner.provider_kind
    }

    /// The bound socket, so health can report the mode that is on disk.
    pub fn remember_socket_path(&self, path: PathBuf) {
        *self.inner.socket_path.lock().expect("socket path") = Some(path);
    }

    /// Four-digit permission bits of the bound socket, or `0600` before bind.
    #[must_use]
    pub fn measured_socket_mode(&self) -> String {
        let path = self.inner.socket_path.lock().expect("socket path").clone();
        let Some(path) = path else {
            return "0600".to_owned();
        };
        std::fs::metadata(path).ok().map_or_else(
            || "0000".to_owned(),
            |metadata| format!("{:04o}", metadata.permissions().mode() & 0o777),
        )
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
    fn publish_authority_change(&self, mut state: MutexGuard<'_, BrokerState>) {
        // Derived text never outlives the authority it was shown under.
        state.type_through = None;
        state.word_start = None;
        let event = state.authority_changed();
        drop(state);
        let _ = self.inner.authority_events.send(event);
    }
}

impl BrokerState {
    /// Paused by the user, while the control plane is degraded or
    /// mid-mutation, while the login session is inactive or asleep, or while
    /// the screen is shared and settings hold suggestions then.
    fn runtime_paused(&self) -> bool {
        self.paused
            || self.control_plane_condition.is_degraded()
            || self.control_plane_mutation_in_progress
            || self.desktop_hold()
    }

    fn effective_paused(&self) -> bool {
        self.runtime_paused()
            || self
                .settings
                .as_ref()
                .is_some_and(|settings| settings.paused)
    }

    fn advance_authority_epoch(&mut self) {
        self.rebound_quiet = None;
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
        // A snooze or an editor's focus withholds this target's grants.
        let withheld = self
            .sessions
            .get(&session_id)
            .ok_or(BrokerError::UnknownSession)
            .map(|session| self.withheld_reason(&session.target.target).is_some())?;
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(BrokerError::UnknownSession)?;
        if withheld
            || self.settings.as_ref().is_some_and(|settings| {
                !authority::settings_allow_data(runtime_paused, settings, &session.target.target)
            })
        {
            return Err(BrokerError::Denied(PolicyReason::PolicyNever));
        }
        Ok(session)
    }

    fn close_all_sessions(&mut self, metrics: &Metrics, reason: ReasonCode) {
        for session in self.sessions.values_mut() {
            session.revoke_context_authority(metrics, reason);
        }
        self.sessions.clear();
        self.type_through = None;
        self.word_start = None;
        self.rebound_quiet = None;
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
