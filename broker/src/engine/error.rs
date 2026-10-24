use thiserror::Error;

use crate::control_plane::ControlPlaneError;
use crate::policy::PolicyReason;

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("commit_lease_expired")]
    CommitLeaseExpired,
    #[error("control_plane")]
    ControlPlane(#[from] ControlPlaneError),
    #[error("control_plane_task")]
    ControlPlaneTask,
    /// The operation is still queued or running; its outcome is unknown.
    #[error("control_plane_timeout")]
    ControlPlaneTimeout,
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
