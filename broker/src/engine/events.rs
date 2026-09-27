use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{BrokerError, PendingCommit, VisibleSuggestion};
use crate::protocol::{
    CommitPreparePayload, Coordinates, MessageType, ProtocolError, ReasonCode,
    SuggestionClearPayload, SuggestionShowPayload, WireEnvelope,
};

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

    pub(super) fn send(&self, event: BrokerEvent) -> Result<(), BrokerError> {
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
    pub fn into_wire(self, mono_ms: u64) -> Result<WireEnvelope, ProtocolError> {
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

    /// Ends a request that produced no visible suggestion.
    pub(super) fn generation_withdrawn(
        coordinates: Coordinates,
        fingerprint: String,
        reason: ReasonCode,
        request_id: Option<String>,
    ) -> Self {
        Self::SuggestionClear {
            coordinates,
            payload: SuggestionClearPayload {
                fingerprint,
                suggestion_id: None,
                reason,
            },
            request_id,
        }
    }
}

impl VisibleSuggestion {
    pub(super) fn into_clear(self, coordinates: Coordinates, reason: ReasonCode) -> BrokerEvent {
        BrokerEvent::SuggestionClear {
            coordinates,
            payload: SuggestionClearPayload {
                fingerprint: self.payload.fingerprint,
                suggestion_id: Some(self.payload.suggestion_id),
                reason,
            },
            request_id: self.request_id,
        }
    }
}

impl PendingCommit {
    pub(super) fn into_clear(self, reason: ReasonCode) -> BrokerEvent {
        BrokerEvent::SuggestionClear {
            coordinates: self.coordinates,
            payload: SuggestionClearPayload {
                fingerprint: self.fingerprint,
                suggestion_id: Some(self.suggestion_id),
                reason,
            },
            request_id: self.request_id,
        }
    }
}
