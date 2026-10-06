//! Accepting or dismissing a shown suggestion, and the adapter's commit result.

use std::time::Instant;

use tokio::time;

use super::{
    Broker, BrokerError, BrokerEvent, PendingCommit, SessionAuthority, SessionState,
    VisibleSuggestion,
};
use crate::metrics::Metrics;
use crate::personalization::PersonalizationSignal;
use crate::policy::PolicyReason;
use crate::protocol::{
    Acceptance, AdapterKind, Capability, CommitPreparePayload, CommitResultPayload, CommitStatus,
    ControlAction, Coordinates, ReasonCode, SessionControlRequestPayload, SuggestRequestPayload,
};

/// The UTC day and signal of one outcome aggregate to record.
type OutcomeSignal = Option<(u64, PersonalizationSignal)>;

impl Broker {
    pub async fn session_control(
        &self,
        coordinates: Coordinates,
        payload: SessionControlRequestPayload,
        request_id: Option<String>,
    ) -> Result<(), BrokerError> {
        payload.validate()?;
        let acceptance = match payload.action {
            ControlAction::Request => {
                let request = SuggestRequestPayload {
                    fingerprint: payload.fingerprint,
                    explicit: true,
                };
                return self
                    .request_suggestion(coordinates, request, request_id)
                    .await;
            }
            ControlAction::Dismiss => None,
            ControlAction::AcceptWord => Some(Acceptance::Word),
            ControlAction::AcceptAll => Some(Acceptance::All),
            ControlAction::Pause | ControlAction::Resume | ControlAction::PauseToggle => {
                return Err(BrokerError::InvalidPayload);
            }
        };

        let mut state = self.inner.state.lock().await;
        if state.effective_paused() {
            return Err(BrokerError::Denied(PolicyReason::Paused));
        }
        let settings_revision = state.settings_revision;
        let session = state.session_with_data_access(coordinates.session_id)?;
        session.ensure_coordinates(coordinates)?;
        session.ensure_fingerprint(&payload.fingerprint)?;
        let suggestion_id = payload
            .suggestion_id
            .as_deref()
            .ok_or(BrokerError::InvalidPayload)?;
        let visible = session.take_live_suggestion(suggestion_id, &self.inner.metrics)?;
        let signal = match acceptance {
            None => self.dismiss(session, visible),
            Some(acceptance) => self.prepare_commit(session, visible, acceptance, request_id)?,
        };
        if let Some((event_day, signal)) = signal {
            let _ =
                self.queue_outcome(&session.target.target, settings_revision, event_day, signal);
        }
        // Escape or a whole acceptance ends type-through; after the next word
        // the remainder may still carry (ADR 0004).
        if acceptance != Some(Acceptance::Word) {
            state.type_through = None;
        }
        Ok(())
    }

    fn dismiss(&self, session: &mut SessionState, visible: VisibleSuggestion) -> OutcomeSignal {
        let aggregate_day = visible.aggregate_day;
        session.generation = session.generation.wrapping_add(1);
        let _ = session
            .sink
            .send(visible.into_clear(session.coordinates, ReasonCode::Dismissed));
        self.inner.metrics.record_dismissal();
        aggregate_day.map(|day| (day, PersonalizationSignal::Dismissed))
    }

    /// Grants the adapter one-shot authority to commit `visible` before the
    /// commit-result lease expires.
    fn prepare_commit(
        &self,
        session: &mut SessionState,
        visible: VisibleSuggestion,
        acceptance: Acceptance,
        request_id: Option<String>,
    ) -> Result<OutcomeSignal, BrokerError> {
        let VisibleSuggestion {
            payload: shown,
            aggregate_day,
            ..
        } = visible;
        let coordinates = session.coordinates;
        let commit = CommitPreparePayload {
            fingerprint: shown.fingerprint.clone(),
            suggestion_id: shown.suggestion_id.clone(),
            text: match acceptance {
                Acceptance::Word => shown.accept_word,
                Acceptance::All => shown.text,
            },
            replace_before: shown.replace_before,
            acceptance,
        };
        let expires_at = Instant::now() + self.inner.config.commit_result_lease;
        let pending = PendingCommit {
            coordinates,
            fingerprint: shown.fingerprint,
            suggestion_id: shown.suggestion_id.clone(),
            request_id: request_id.clone(),
            expires_at,
        };
        let prepare = BrokerEvent::CommitPrepare {
            coordinates,
            payload: commit,
            request_id,
        };
        if session.sink.send(prepare).is_err() {
            self.inner.metrics.record_commit_failure();
            return Err(BrokerError::EventSinkClosed);
        }
        session.pending = Some(pending);
        self.renew_context_authority_lease(session, self.inner.config.generation_timeout);
        self.inner.metrics.record_commit_prepared();

        let broker = self.clone();
        let suggestion_id = shown.suggestion_id;
        tokio::spawn(async move {
            time::sleep(broker.inner.config.commit_result_lease).await;
            broker
                .expire_pending_commit(coordinates, suggestion_id, expires_at)
                .await;
        });
        let signal = match acceptance {
            Acceptance::Word => PersonalizationSignal::AcceptedWord,
            Acceptance::All => PersonalizationSignal::AcceptedAll,
        };
        Ok(aggregate_day.map(|day| (day, signal)))
    }

    // Commit lease validation and state retirement are one atomic audit path.
    pub async fn commit_result(
        &self,
        coordinates: Coordinates,
        payload: CommitResultPayload,
    ) -> Result<(), BrokerError> {
        payload.validate()?;
        let metrics = &self.inner.metrics;
        let mut state = self.inner.state.lock().await;
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        session.ensure_coordinates(coordinates)?;
        let pending = session
            .pending
            .as_ref()
            .ok_or(BrokerError::NoPendingCommit)?;
        if pending.coordinates != coordinates
            || pending.fingerprint != payload.fingerprint
            || pending.suggestion_id != payload.suggestion_id
        {
            metrics.record_commit_failure();
            return Err(BrokerError::Stale);
        }
        if pending.expires_at <= Instant::now() {
            let pending = session.pending.take().expect("pending commit checked");
            let _ = session.sink.send(pending.into_clear(ReasonCode::Expired));
            metrics.record_commit_failure();
            return Err(BrokerError::CommitLeaseExpired);
        }
        validate_commit_authority(&session.authority, payload.status)?;
        session.pending = None;
        retire_committed_context(session, payload.status, metrics);
        Ok(())
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
        let Some(pending) = session.pending.take_if(|pending| {
            pending.coordinates == coordinates
                && pending.suggestion_id == suggestion_id
                && pending.expires_at == expires_at
                && pending.expires_at <= Instant::now()
        }) else {
            return;
        };
        let _ = session.sink.send(pending.into_clear(ReasonCode::Expired));
        self.inner.metrics.record_commit_failure();
    }
}

impl SessionState {
    /// Takes the visible suggestion `suggestion_id` unless it has expired, in
    /// which case the adapter is told to clear it.
    fn take_live_suggestion(
        &mut self,
        suggestion_id: &str,
        metrics: &Metrics,
    ) -> Result<VisibleSuggestion, BrokerError> {
        let visible = self.visible.as_ref().ok_or(BrokerError::NoSuggestion)?;
        if visible.payload.suggestion_id != suggestion_id {
            return Err(BrokerError::Stale);
        }
        let expired = visible.expires_at <= Instant::now();
        let visible = self.visible.take().expect("visible suggestion checked");
        if expired {
            let _ = self
                .sink
                .send(visible.into_clear(self.coordinates, ReasonCode::Expired));
            metrics.record_suggestion_expired();
            return Err(BrokerError::NoSuggestion);
        }
        Ok(visible)
    }
}

fn retire_committed_context(session: &mut SessionState, status: CommitStatus, metrics: &Metrics) {
    match status {
        CommitStatus::Applied => {
            // A commit mutates the document and retires every byte derived
            // from the old revision. The adapter must provide fresh context
            // and request a newly generated continuation; a cached suffix
            // cannot inherit authority or restart a relative display TTL.
            session.invalidate_context_lease();
            session.generation = session.generation.wrapping_add(1);
            session.cancel_provider_work();
            session.context = None;
            session.visible = None;
            metrics.record_commit_applied();
        }
        CommitStatus::DispatchedUnverified => {
            session.invalidate_context_lease();
            session.context = None;
        }
        CommitStatus::Stale | CommitStatus::Blocked | CommitStatus::Failed => {
            // The adapter's terminal result says the pre-commit document
            // authority is no longer usable. Do not retain that context
            // for another request until the adapter supplies newer state.
            session.revoke_context_authority(metrics, ReasonCode::Stale);
            metrics.record_commit_failure();
        }
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
                AdapterKind::Browser | AdapterKind::Obsidian | AdapterKind::Terminal
            ) && authority.grants(Capability::CommitApplied)
        }
        CommitStatus::DispatchedUnverified => {
            authority.adapter_kind == AdapterKind::Fcitx
                && authority.grants(Capability::CommitDispatchedUnverified)
        }
        CommitStatus::Stale | CommitStatus::Blocked | CommitStatus::Failed => true,
    };
    if allowed {
        Ok(())
    } else {
        Err(BrokerError::InvalidCapability)
    }
}

#[cfg(test)]
mod tests {
    use super::{BrokerError, SessionAuthority, validate_commit_authority};
    use crate::protocol::{AdapterKind, Capability, CommitStatus};

    #[test]
    fn commit_status_requires_matching_adapter_kind_and_declared_capability() {
        let browser_dispatch = SessionAuthority {
            protocol_version: 1,
            adapter_kind: AdapterKind::Browser,
            capabilities: vec![Capability::CommitDispatchedUnverified],
        };
        for status in [CommitStatus::DispatchedUnverified, CommitStatus::Applied] {
            assert!(matches!(
                validate_commit_authority(&browser_dispatch, status),
                Err(BrokerError::InvalidCapability)
            ));
        }

        let browser_applied = SessionAuthority {
            protocol_version: 1,
            adapter_kind: AdapterKind::Browser,
            capabilities: vec![Capability::CommitApplied],
        };
        assert!(validate_commit_authority(&browser_applied, CommitStatus::Applied).is_ok());

        let browser_undeclared = SessionAuthority {
            protocol_version: 1,
            adapter_kind: AdapterKind::Browser,
            capabilities: Vec::new(),
        };
        assert!(matches!(
            validate_commit_authority(&browser_undeclared, CommitStatus::DispatchedUnverified),
            Err(BrokerError::InvalidCapability)
        ));

        let obsidian_wrong_status = SessionAuthority {
            protocol_version: 1,
            adapter_kind: AdapterKind::Obsidian,
            capabilities: vec![Capability::CommitDispatchedUnverified],
        };
        assert!(matches!(
            validate_commit_authority(&obsidian_wrong_status, CommitStatus::DispatchedUnverified,),
            Err(BrokerError::InvalidCapability)
        ));

        let fcitx_dispatch = SessionAuthority {
            protocol_version: 2,
            adapter_kind: AdapterKind::Fcitx,
            capabilities: vec![Capability::CommitDispatchedUnverified],
        };
        assert!(
            validate_commit_authority(&fcitx_dispatch, CommitStatus::DispatchedUnverified).is_ok()
        );
    }
}
