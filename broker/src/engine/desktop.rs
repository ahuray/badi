//! Desktop session helper reports: a login session that is not in the
//! foreground (or a machine going to sleep) holds every grant, and battery
//! power or the power-saver profile changes how automatic requests start.
//!
//! A report never grants authority. Each one is bound to the connection that
//! sent it, so a helper that exits takes its hold and low-power state with it.

use std::time::Duration;

use tokio::sync::MutexGuard;

use super::{Broker, BrokerError, BrokerState};
use crate::protocol::{
    AutomaticScheduling, DesktopSession, DesktopStatePayload, DesktopStatusPayload, ReasonCode,
};
use crate::provider::RequestTrigger;
use crate::settings::BatteryMode;

/// In quiet battery mode an automatic request waits this long before the
/// model starts; any newer request or edit cancels it first, so keys typed
/// closer together never start the model. On the 2026-10-13 power-saver
/// trial a 120 ms wait (the Fcitx display pause) still started the model for
/// every key typed 150 ms apart, so this is a real typing pause instead.
pub const QUIET_START_DELAY: Duration = Duration::from_millis(300);

impl Broker {
    /// Records one helper connection's report. Entering or leaving a held
    /// session advances the authority epoch: entering it revokes context,
    /// pending commits and the type-through carry, and leaving it makes every
    /// adapter acquire the field afresh.
    pub async fn report_desktop_state(
        &self,
        connection_id: &str,
        report: DesktopStatePayload,
    ) -> Result<DesktopStatusPayload, BrokerError> {
        let mut state = self.inner.state.lock().await;
        let was_held = state.desktop_hold();
        state.desktop.insert(connection_id.to_owned(), report);
        self.finish_desktop_transition(state, was_held).await
    }

    /// Drops a closed helper connection's report and any hold it placed.
    pub async fn forget_desktop_state(&self, connection_id: &str) {
        let mut state = self.inner.state.lock().await;
        let was_held = state.desktop_hold();
        if state.desktop.remove(connection_id).is_none() {
            return;
        }
        // The connection is gone; a failed outcome fence was already logged.
        let _ = self.finish_desktop_transition(state, was_held).await;
    }

    pub async fn desktop_status(&self) -> DesktopStatusPayload {
        self.inner.state.lock().await.desktop_status()
    }

    async fn finish_desktop_transition(
        &self,
        mut state: MutexGuard<'_, BrokerState>,
        was_held: bool,
    ) -> Result<DesktopStatusPayload, BrokerError> {
        let status = state.desktop_status();
        let held = state.desktop_hold();
        if held == was_held {
            return Ok(status);
        }
        state.advance_authority_epoch();
        if held {
            for session in state.sessions.values_mut() {
                session.revoke_context_authority(&self.inner.metrics, ReasonCode::Paused);
            }
        }
        self.publish_authority_change(state);
        if held {
            self.flush_outcomes_before_pause_ack().await?;
        }
        Ok(status)
    }
}

impl BrokerState {
    /// Any helper reports an inactive or sleeping session.
    pub(super) fn desktop_hold(&self) -> bool {
        self.desktop.values().any(|report| report.session.holds())
    }

    fn low_power(&self) -> bool {
        self.desktop
            .values()
            .any(|report| report.on_battery || report.power_saver)
    }

    fn automatic_scheduling(&self) -> AutomaticScheduling {
        if !self.low_power() {
            return AutomaticScheduling::Normal;
        }
        match self
            .settings
            .as_ref()
            .map_or_else(BatteryMode::default, |settings| settings.battery_mode)
        {
            BatteryMode::Normal => AutomaticScheduling::Normal,
            BatteryMode::Quiet => AutomaticScheduling::Deferred,
            BatteryMode::ExplicitOnly => AutomaticScheduling::ExplicitOnly,
        }
    }

    pub(super) fn desktop_status(&self) -> DesktopStatusPayload {
        let session = if self
            .desktop
            .values()
            .any(|report| report.session == DesktopSession::Sleeping)
        {
            DesktopSession::Sleeping
        } else if self.desktop_hold() {
            DesktopSession::Inactive
        } else {
            DesktopSession::Active
        };
        DesktopStatusPayload {
            observed: !self.desktop.is_empty(),
            session,
            low_power: self.low_power(),
            automatic: self.automatic_scheduling(),
        }
    }

    /// How long an admitted request waits before the model starts, or `None`
    /// when it must abstain. Explicit requests are never deferred or refused.
    pub(super) fn power_start_delay(&self, trigger: RequestTrigger) -> Option<Duration> {
        if trigger == RequestTrigger::Explicit {
            return Some(Duration::ZERO);
        }
        match self.automatic_scheduling() {
            AutomaticScheduling::Normal => Some(Duration::ZERO),
            AutomaticScheduling::Deferred => Some(QUIET_START_DELAY),
            AutomaticScheduling::ExplicitOnly => None,
        }
    }
}
