//! Pause, authority epochs, policy subscribers, and target policy resolution.

use tokio::sync::{MutexGuard, broadcast};

use super::{Broker, BrokerError, BrokerState};
use crate::metrics::Metrics;
use crate::protocol::{
    Activation, AuthorityChangedPayload, PolicyResolutionReason, PolicyStatusPayload, ReasonCode,
    TargetDescriptor,
};
use crate::settings::{PolicyResolution, SettingsV1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthoritySnapshot {
    pub authority_epoch: u64,
    pub settings_revision: u64,
    pub paused: bool,
    pub control_plane_degraded: bool,
    pub pending_acknowledgements: usize,
}

impl Broker {
    pub async fn set_paused(&self, paused: bool) -> bool {
        let mut state = self.inner.state.lock().await;
        let changed = state.set_runtime_pause(paused, &self.inner.metrics);
        self.finish_pause_transition(state, changed).await
    }

    pub async fn toggle_paused(&self) -> bool {
        let mut state = self.inner.state.lock().await;
        let paused = !state.paused;
        let changed = state.set_runtime_pause(paused, &self.inner.metrics);
        self.finish_pause_transition(state, changed).await
    }

    /// Publishes a changed epoch and, while paused, fences queued outcome
    /// writes before the pause is acknowledged.
    async fn finish_pause_transition(
        &self,
        state: MutexGuard<'_, BrokerState>,
        changed: bool,
    ) -> bool {
        let effective_paused = state.effective_paused();
        if changed {
            self.publish_authority_change(state);
        } else {
            drop(state);
        }
        if effective_paused {
            self.flush_outcomes_before_pause_ack().await;
        }
        self.is_paused().await
    }

    pub async fn is_paused(&self) -> bool {
        self.inner.state.lock().await.effective_paused()
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
        let acknowledged = state
            .policy_clients
            .get_mut(connection_id)
            .ok_or(BrokerError::InvalidCapability)?;
        *acknowledged =
            Some(acknowledged.map_or(authority_epoch, |value| value.max(authority_epoch)));
        Ok(())
    }

    pub async fn authority_snapshot(&self) -> AuthoritySnapshot {
        let state = self.inner.state.lock().await;
        AuthoritySnapshot {
            authority_epoch: state.authority_epoch,
            settings_revision: state.settings_revision,
            paused: state.effective_paused(),
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
                state.runtime_paused(),
                state.authority_epoch,
                settings,
                target,
            )
        } else {
            denied_policy_status(
                state.authority_epoch,
                state.settings_revision,
                state.runtime_paused(),
                PolicyResolutionReason::UnknownIdentity,
            )
        }
    }
}

impl BrokerState {
    /// Returns whether the runtime pause changed. Once effectively paused,
    /// every session loses its context authority.
    fn set_runtime_pause(&mut self, paused: bool, metrics: &Metrics) -> bool {
        if self.paused == paused {
            return false;
        }
        self.paused = paused;
        self.advance_authority_epoch();
        if self.effective_paused() {
            for session in self.sessions.values_mut() {
                session.revoke_context_authority(metrics, ReasonCode::Paused);
            }
        }
        true
    }
}

pub(super) fn settings_allow_data(
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

pub(super) fn policy_status(
    runtime_paused: bool,
    authority_epoch: u64,
    settings: &SettingsV1,
    target: &TargetDescriptor,
) -> PolicyStatusPayload {
    let resolution = settings.resolve_target_validated(target);
    if runtime_paused || resolution.paused {
        return denied_policy_status(
            authority_epoch,
            settings.revision,
            true,
            PolicyResolutionReason::GlobalDisabled,
        );
    }
    if let Some(reason) = denial_reason(resolution) {
        return denied_policy_status(authority_epoch, settings.revision, false, reason);
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

fn denial_reason(resolution: PolicyResolution) -> Option<PolicyResolutionReason> {
    if !resolution.identity_known {
        Some(PolicyResolutionReason::UnknownIdentity)
    } else if !resolution.configured {
        Some(PolicyResolutionReason::DefaultPolicy)
    } else if !resolution.allows_context_read() {
        Some(PolicyResolutionReason::ContextDisabled)
    } else if !resolution.allows_display() || !resolution.allows_suggestion() {
        Some(PolicyResolutionReason::SuggestionsDisabled)
    } else {
        None
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
