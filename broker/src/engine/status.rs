use std::time::Instant;

use super::{Broker, BrokerState};
use crate::metrics::MetricsSnapshot;
use crate::protocol::{ActiveLocator, MAX_FRAME_BYTES, ProviderKind};

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

impl Broker {
    pub async fn session_count(&self) -> u64 {
        u64::try_from(self.inner.state.lock().await.sessions.len()).unwrap_or(u64::MAX)
    }

    pub async fn active_locator(&self) -> Option<ActiveLocator> {
        self.inner.state.lock().await.active_locator()
    }

    pub async fn health_snapshot(&self) -> HealthSnapshot {
        let state = self.inner.state.lock().await;
        HealthSnapshot {
            provider: self.provider_kind(),
            paused: state.effective_paused(),
            authority_epoch: state.authority_epoch,
            settings_revision: state.settings_revision,
            control_plane_degraded: state.control_plane_condition.is_degraded(),
            sessions: u64::try_from(state.sessions.len()).unwrap_or(u64::MAX),
            max_frame_bytes: MAX_FRAME_BYTES,
            metrics: self.inner.metrics.snapshot(),
            active: state.active_locator(),
        }
    }
}

impl BrokerState {
    /// The one focused session with context. Two focused claimants are
    /// ambiguous, so neither is addressable.
    fn active_locator(&self) -> Option<ActiveLocator> {
        if self.effective_paused() {
            return None;
        }
        let mut candidates = self.sessions.values().filter_map(|session| {
            let context = session.context.as_ref()?;
            if !context.field.focused {
                return None;
            }
            Some(ActiveLocator {
                session_id: session.coordinates.session_id,
                focus_epoch: session.coordinates.focus_epoch,
                revision: session.coordinates.revision,
                fingerprint: context.fingerprint.clone(),
                suggestion_id: session
                    .visible
                    .as_ref()
                    .filter(|visible| visible.expires_at > Instant::now())
                    .map(|visible| visible.payload.suggestion_id.clone()),
            })
        });
        let only = candidates.next()?;
        candidates.next().is_none().then_some(only)
    }
}
