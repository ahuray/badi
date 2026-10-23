//! Session lifecycle and the context authority each session holds.

use std::time::{Duration, Instant};

use tokio::time;
use tokio_util::sync::CancellationToken;

use super::authority::policy_status;
use super::{
    Broker, BrokerError, BrokerEventSink, BrokerState, ContextOutcome, SessionAuthority,
    SessionState,
};
use crate::metrics::Metrics;
use crate::policy::{PolicyDecision, PolicyInput, PolicyReason, evaluate};
use crate::protocol::{
    Activation, Capability, ContextChangedPayload, Coordinates, ReasonCode, SessionId,
    SessionOpenPayload, TargetKind,
};
use crate::provider::RequestTrigger;

impl Broker {
    pub async fn open_session(
        &self,
        coordinates: Coordinates,
        payload: SessionOpenPayload,
        authority: SessionAuthority,
        sink: BrokerEventSink,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_coordinate_bounds(coordinates.focus_epoch, coordinates.revision)?;
        payload.target.validate()?;
        if !authority.grants(Capability::Context) || !authority.grants(Capability::Suggestion) {
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
            if !authority.grants(Capability::Policy) {
                return Err(BrokerError::InvalidCapability);
            }
            let policy = policy_status(
                state.runtime_paused(),
                state.withheld_reason(&payload.target),
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
            SessionState::new(coordinates, payload, authority, sink),
        );
        Ok(())
    }

    pub async fn close_session(&self, coordinates: Coordinates) -> Result<(), BrokerError> {
        let mut state = self.inner.state.lock().await;
        state
            .sessions
            .get(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?
            .ensure_coordinates(coordinates)?;
        let mut session = state
            .sessions
            .remove(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        session.revoke_context_authority(&self.inner.metrics, ReasonCode::SessionClosed);
        Ok(())
    }

    pub async fn close_owned_sessions(&self, session_ids: &[SessionId]) {
        let mut state = self.inner.state.lock().await;
        for session_id in session_ids {
            if let Some(mut session) = state.sessions.remove(session_id) {
                session.revoke_context_authority(&self.inner.metrics, ReasonCode::SessionClosed);
            }
        }
    }

    /// Retires every session and cancels all provider work before server shutdown.
    pub async fn shutdown(&self) {
        // Closing admission is terminal and nonblocking: no request racing
        // shutdown can acquire new provider capacity after this point.
        self.inner.provider_admissions.close();
        self.inner.shutdown.cancel();
        self.inner
            .state
            .lock()
            .await
            .close_all_sessions(&self.inner.metrics, ReasonCode::SessionClosed);
        self.flush_outcomes_before_shutdown().await;
    }

    pub async fn update_context(
        &self,
        coordinates: Coordinates,
        mut payload: ContextChangedPayload,
    ) -> Result<ContextOutcome, BrokerError> {
        crate::protocol::validate_coordinate_bounds(coordinates.focus_epoch, coordinates.revision)?;
        let metrics = &self.inner.metrics;
        let mut state = self.inner.state.lock().await;
        let paused = state.effective_paused();
        let language = state
            .sessions
            .get(&coordinates.session_id)
            .and_then(|session| {
                state
                    .settings
                    .as_ref()?
                    .language_override(&session.target.target)
            });
        let session = state.session_with_data_access(coordinates.session_id)?;
        session.ensure_newer_context(coordinates)?;
        // Apply broker-owned activation before validating content. A restrictive
        // session must reject non-empty context even when an adapter claims a
        // more permissive activation in this individual update.
        payload.activation = restrictive_activation(session.target.activation, payload.activation);
        payload.validate()?;
        metrics.record_context_update();
        session.revoke_context_authority(metrics, ReasonCode::Superseded);
        session.coordinates = coordinates;
        session.context_seen_at_coordinates = true;

        match evaluate_context(
            &payload,
            payload.explicit,
            session.target.target.kind,
            paused,
        ) {
            PolicyDecision::Allow(_) => {
                if let Some(language) = language
                    .map(crate::settings::PredictionLanguage::tag)
                    .or_else(|| crate::language::detect_field_language(&payload.before))
                {
                    payload.language = Some(language.to_owned());
                }
                session.context = Some(payload);
                self.renew_context_authority_lease(session, self.inner.config.generation_timeout);
                Ok(ContextOutcome::Allowed)
            }
            PolicyDecision::ManualRequired(_) => {
                metrics.record_manual_required();
                Ok(ContextOutcome::ManualRequired)
            }
            PolicyDecision::Deny(_) => {
                metrics.record_denied();
                Ok(ContextOutcome::Denied)
            }
        }
    }

    pub(super) fn renew_context_authority_lease(
        &self,
        session: &mut SessionState,
        generation_timeout: Duration,
    ) {
        session.invalidate_context_lease();
        let lease = self
            .inner
            .config
            .context_authority_lease_for(&session.authority, generation_timeout);
        let session_id = session.coordinates.session_id;
        let lease_generation = session.context_lease_generation;
        let cancellation = CancellationToken::new();
        session.context_lease_cancellation = Some(cancellation.clone());
        let broker = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = time::sleep(lease) => {
                    broker.expire_context_authority(session_id, lease_generation).await;
                }
                () = cancellation.cancelled() => {}
            }
        });
    }

    async fn expire_context_authority(&self, session_id: SessionId, lease_generation: u64) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&session_id) else {
            return;
        };
        if session.context_lease_generation == lease_generation
            && session.context_lease_cancellation.is_some()
        {
            session.revoke_context_authority(&self.inner.metrics, ReasonCode::Expired);
        }
    }
}

/// Fcitx apps whose spelling another channel owns: `libreoffice`, whose
/// Writer extension corrects ([ADR 0003]). Every other observed field may get a correction,
/// which the Fcitx addon applies only after the observer confirms the exact
/// field and then reads back ([ADR 0011]).
fn fcitx_spelling_elsewhere(app_id: &str) -> bool {
    app_id == "libreoffice"
}

impl SessionState {
    pub(super) fn observed_quiet_target(&self) -> Option<crate::protocol::TargetDescriptor> {
        (self.authority.adapter_kind == crate::protocol::AdapterKind::Fcitx
            && self
                .context
                .as_ref()
                .is_some_and(|context| context.field.identity_known))
        .then(|| self.target.target.clone())
    }

    fn new(
        coordinates: Coordinates,
        target: SessionOpenPayload,
        authority: SessionAuthority,
        sink: BrokerEventSink,
    ) -> Self {
        Self {
            coordinates,
            target,
            authority,
            context: None,
            context_seen_at_coordinates: false,
            context_lease_generation: 0,
            context_lease_cancellation: None,
            generation: 0,
            cancellation: None,
            visible: None,
            pending: None,
            escape_quiet: None,
            sink,
        }
    }

    /// Whether a dismissal's timed quiet binds this request. Asking
    /// explicitly ends the quiet at once.
    pub(super) fn escape_quiet_binds(
        &mut self,
        explicit: bool,
        focus_epoch: u64,
        now: Instant,
    ) -> bool {
        if explicit {
            self.escape_quiet = None;
            return false;
        }
        self.escape_quiet
            .is_some_and(|quiet| quiet.active(focus_epoch, now))
    }

    pub(super) fn context_fingerprint(&self) -> Option<&str> {
        self.context
            .as_ref()
            .map(|context| context.fingerprint.as_str())
    }

    /// A spelling correction needs negotiated replacement and a field whose
    /// identity the adapter knows. IME-parity apps stay append-only even when
    /// the shared Fcitx connection advertises replacement for native fields.
    /// The Writer extension shows corrections only, so its requests never
    /// run a continuation.
    pub(super) fn spelling_only(&self) -> bool {
        self.target.target.kind == TargetKind::Libreoffice
    }

    /// What a request asks the provider for, and with which budget.
    pub(super) fn trigger(&self, explicit: bool) -> RequestTrigger {
        if self.spelling_only() {
            RequestTrigger::Spelling
        } else {
            RequestTrigger::from_explicit(explicit)
        }
    }

    /// A spelling-only field without a misspelled word calls no provider.
    pub(super) fn nothing_to_correct(&self, explicit: bool) -> bool {
        self.spelling_only()
            && self.context.as_ref().is_none_or(|context| {
                crate::spelling::global()
                    .locate(&context.before, context.language.as_deref(), explicit)
                    .is_none()
            })
    }

    pub(super) fn allows_text_replacement(&self) -> bool {
        self.authority.grants(Capability::TextReplacement)
            && self
                .context
                .as_ref()
                .is_some_and(|context| context.field.identity_known)
            && !fcitx_spelling_elsewhere(&self.target.target.app_id)
    }

    pub(super) fn ensure_coordinates(&self, coordinates: Coordinates) -> Result<(), BrokerError> {
        if self.coordinates == coordinates {
            Ok(())
        } else {
            Err(BrokerError::Stale)
        }
    }

    /// Matches the current context, or the visible suggestion once context
    /// authority has lapsed.
    pub(super) fn ensure_fingerprint(&self, fingerprint: &str) -> Result<(), BrokerError> {
        let current = self.context_fingerprint().or_else(|| {
            self.visible
                .as_ref()
                .map(|visible| visible.payload.fingerprint.as_str())
        });
        if current == Some(fingerprint) {
            Ok(())
        } else {
            Err(BrokerError::Stale)
        }
    }

    fn ensure_newer_context(&self, coordinates: Coordinates) -> Result<(), BrokerError> {
        let current = self.coordinates;
        let is_older = (coordinates.focus_epoch, coordinates.revision)
            < (current.focus_epoch, current.revision);
        let is_repeat = coordinates.focus_epoch == current.focus_epoch
            && coordinates.revision == current.revision
            && self.context_seen_at_coordinates;
        if is_older || is_repeat {
            Err(BrokerError::Stale)
        } else {
            Ok(())
        }
    }

    pub(super) fn invalidate_context_lease(&mut self) {
        self.context_lease_generation = self.context_lease_generation.wrapping_add(1);
        if let Some(cancellation) = self.context_lease_cancellation.take() {
            cancellation.cancel();
        }
    }

    pub(super) fn revoke_context_authority(&mut self, metrics: &Metrics, reason: ReasonCode) {
        self.invalidate_context_lease();
        self.retire_generation(metrics, reason);
        self.context = None;
    }

    /// Ends the current generation: cancels its provider work and withdraws
    /// its visible suggestion or pending commit.
    pub(super) fn retire_generation(&mut self, metrics: &Metrics, reason: ReasonCode) {
        self.generation = self.generation.wrapping_add(1);
        let provider_cancelled = self.cancel_provider_work();
        let pending = self.pending.take();
        let commit_cancelled = pending.is_some();
        let clear = match (self.visible.take(), pending) {
            (Some(visible), _) => Some(visible.into_clear(self.coordinates, reason)),
            (None, Some(pending)) => Some(pending.into_clear(reason)),
            (None, None) => None,
        };
        if let Some(clear) = clear {
            let _ = self.sink.send(clear);
        }
        if provider_cancelled || commit_cancelled {
            metrics.record_cancellation();
        }
    }

    pub(super) fn cancel_provider_work(&mut self) -> bool {
        let Some(cancellation) = self.cancellation.take() else {
            return false;
        };
        cancellation.cancel();
        true
    }
}

/// `explicit` is whether the user asked for this particular context or request.
pub(super) fn evaluate_context(
    context: &ContextChangedPayload,
    explicit: bool,
    target_kind: TargetKind,
    paused: bool,
) -> PolicyDecision {
    evaluate(PolicyInput {
        activation: context.activation,
        explicit,
        field: context.field,
        target_kind,
        paused,
        selection_collapsed: context.selection.anchor == context.selection.head,
    })
}

const fn restrictive_activation(configured: Activation, claimed: Activation) -> Activation {
    match (configured, claimed) {
        (Activation::Never, _) | (_, Activation::Never) => Activation::Never,
        (Activation::Manual, _) | (_, Activation::Manual) => Activation::Manual,
        (Activation::Always, Activation::Always) => Activation::Always,
    }
}

impl BrokerState {
    pub(super) fn quiet_request(
        &mut self,
        coordinates: Coordinates,
        explicit: bool,
    ) -> Result<bool, BrokerError> {
        let now = Instant::now();
        let session = self.session_with_data_access(coordinates.session_id)?;
        let local = session.escape_quiet_binds(explicit, coordinates.focus_epoch, now);
        let target = session.observed_quiet_target();
        let rebound = self
            .rebound_quiet
            .as_ref()
            .is_some_and(|quiet| target.as_ref() == Some(&quiet.target) && now < quiet.until);
        if (explicit && rebound)
            || self
                .rebound_quiet
                .as_ref()
                .is_some_and(|quiet| now >= quiet.until)
        {
            self.rebound_quiet = None;
        }
        Ok(local || (rebound && !explicit))
    }
}
