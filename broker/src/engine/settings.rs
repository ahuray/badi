//! Settings replacement and memory clearing through the control plane. The
//! data plane stays paused while a mutation's outcome is unknown.

use std::time::Duration;

use tokio::time;

use super::{Broker, BrokerError, BrokerState, ControlPlaneCondition};
use crate::control_plane::{ControlPlane, ControlPlaneError, ControlPlaneSnapshot};
use crate::personalization::PersonalizationStoreError;
use crate::protocol::ReasonCode;
use crate::settings::{PrivateStorageError, SettingsStoreError, SettingsV1};

const PERSONALIZATION_CLEAR_TIMEOUT: Duration = Duration::from_secs(5);

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

impl Broker {
    pub async fn control_plane_snapshot(&self) -> Result<ControlPlaneSnapshot, BrokerError> {
        self.inner
            .outcome_recorder
            .clone()
            .ok_or(BrokerError::ControlPlaneUnavailable)?
            .snapshot()
            .await
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
        let outcome = tokio::task::spawn_blocking(move || {
            commit_settings(&control_plane, expected_revision, next)
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
        // Outcomes are queued only under the broker state lock. Queueing the
        // clear under it places the recorder's FIFO barrier after every
        // pre-clear outcome and before every later one, and visible
        // suggestions lose their link to a Shown aggregate about to vanish.
        // The disk work then runs without blocking suggestion traffic.
        let response = {
            let mut state = self.inner.state.lock().await;
            for session in state.sessions.values_mut() {
                if let Some(visible) = session.visible.as_mut() {
                    visible.aggregate_day = None;
                }
            }
            recorder.begin_clear()?
        };
        let (changed, snapshot) = time::timeout(PERSONALIZATION_CLEAR_TIMEOUT, response)
            .await
            .map_err(|_| BrokerError::ControlPlaneTimeout)?
            .map_err(|_| BrokerError::ControlPlaneTask)??;
        self.recover_after_clear(&snapshot.settings).await;
        Ok((changed, snapshot))
    }

    /// A successful clear ends a recoverable degradation, never a
    /// restart-required one.
    async fn recover_after_clear(&self, settings: &SettingsV1) {
        let mut state = self.inner.state.lock().await;
        if state.control_plane_condition != ControlPlaneCondition::Recoverable {
            return;
        }
        state.control_plane_condition = ControlPlaneCondition::Healthy;
        state.install_settings(settings.clone());
        state.advance_authority_epoch();
        self.publish_authority_change(state);
    }

    async fn install_settings_authority(&self, settings: SettingsV1, degraded: bool) {
        let mut state = self.inner.state.lock().await;
        state.close_all_sessions(&self.inner.metrics, ReasonCode::PolicyNever);
        state.control_plane_mutation_in_progress = false;
        state.control_plane_condition = if degraded {
            ControlPlaneCondition::Recoverable
        } else {
            ControlPlaneCondition::Healthy
        };
        state.install_settings(settings);
        state.advance_authority_epoch();
        self.publish_authority_change(state);
    }

    async fn begin_control_plane_mutation(&self) {
        let mut state = self.inner.state.lock().await;
        state.close_all_sessions(&self.inner.metrics, ReasonCode::PolicyNever);
        state.control_plane_mutation_in_progress = true;
        state.advance_authority_epoch();
        self.publish_authority_change(state);
    }

    async fn reject_control_plane_mutation(&self) {
        let mut state = self.inner.state.lock().await;
        state.control_plane_mutation_in_progress = false;
        state.advance_authority_epoch();
        self.publish_authority_change(state);
    }

    pub(super) async fn fail_control_plane_mutation_unknown(&self) {
        let mut state = self.inner.state.lock().await;
        state.control_plane_mutation_in_progress = false;
        state.control_plane_condition = ControlPlaneCondition::RestartRequired;
        state.advance_authority_epoch();
        self.publish_authority_change(state);
    }

    pub(super) async fn fail_control_plane_mutation_recoverable(&self) {
        let mut state = self.inner.state.lock().await;
        state.control_plane_mutation_in_progress = false;
        // Never downgrade an earlier commit-unknown state. Only a coherent
        // settings installation (or process restart/reload) may clear it.
        if state.control_plane_condition == ControlPlaneCondition::Healthy {
            state.control_plane_condition = ControlPlaneCondition::Recoverable;
        }
        state.advance_authority_epoch();
        self.publish_authority_change(state);
    }
}

impl BrokerState {
    fn install_settings(&mut self, settings: SettingsV1) {
        self.settings_revision = settings.revision;
        self.settings = Some(settings);
    }
}

/// Commits `next` and classifies the result by what it means for the data plane.
fn commit_settings(
    control_plane: &ControlPlane,
    expected_revision: u64,
    next: SettingsV1,
) -> SettingsReplaceOutcome {
    let committed = next.clone();
    match control_plane.replace_settings(expected_revision, next) {
        Ok(_) => match control_plane.snapshot() {
            Ok(snapshot) => SettingsReplaceOutcome::Updated(snapshot),
            Err(error) => SettingsReplaceOutcome::CommittedDegraded {
                settings: committed,
                error,
            },
        },
        Err(error @ ControlPlaneError::SettingsCommittedReconciliation { .. }) => {
            SettingsReplaceOutcome::CommittedDegraded {
                settings: committed,
                error,
            }
        }
        Err(error @ ControlPlaneError::Settings(SettingsStoreError::CommitStateUnknown)) => {
            SettingsReplaceOutcome::CommitUnknown(error)
        }
        Err(
            error @ ControlPlaneError::Personalization(PersonalizationStoreError::Storage(
                PrivateStorageError::CommitStateUnknown,
            )),
        ) => SettingsReplaceOutcome::RejectedDegraded(error),
        Err(error) => SettingsReplaceOutcome::Rejected(error),
    }
}
