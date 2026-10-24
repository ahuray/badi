//! Desktop session helper reports: a login session that is not in the
//! foreground (or a machine going to sleep) holds every grant, a live screen
//! share holds them too unless settings keep suggesting, and battery power or
//! the power-saver profile changes how automatic requests start.
//!
//! A report never grants authority. Each one is bound to the connection that
//! sent it, so a helper that exits takes its hold and low-power state with it,
//! except a live screen share: that stays held for a bounded grace, so a
//! helper that crashes or restarts mid-share never lets grey text onto the
//! shared screen while it comes back.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tokio::sync::MutexGuard;

use super::{Broker, BrokerError, BrokerInner, BrokerState};
use crate::protocol::{
    AutomaticScheduling, DesktopSession, DesktopStatePayload, DesktopStatusPayload, ReasonCode,
    ScreenShare,
};
use crate::provider::RequestTrigger;
use crate::settings::{BatteryMode, ScreenShareMode};

/// In quiet battery mode an automatic request waits this long before the
/// model starts; any newer request or edit cancels it first, so keys typed
/// closer together never start the model. The 120 ms Fcitx display pause was
/// too short: it still started the model for keys typed 150 ms apart.
pub const QUIET_START_DELAY: Duration = Duration::from_millis(300);

/// How long a share stays held after its helper disconnected mid-share,
/// unless a helper reports again sooner: systemd restarts a failed helper
/// after 3 s, and its watcher sees a live stream again within a second.
pub const SCREEN_SHARE_GRACE: Duration = Duration::from_secs(10);

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
        // A fresh report says what is shared now, so it ends any grace.
        state.share_grace = None;
        state.desktop.insert(connection_id.to_owned(), report);
        self.finish_desktop_transition(state, was_held).await
    }

    /// Drops a closed helper connection's report and any hold it placed. A
    /// connection that last reported a live share leaves that share held for
    /// `screen_share_grace`, or until a helper reports again.
    pub async fn forget_desktop_state(&self, connection_id: &str) {
        let mut state = self.inner.state.lock().await;
        let was_held = state.desktop_hold();
        let Some(report) = state.desktop.remove(connection_id) else {
            return;
        };
        if report.screen_shared {
            let until = Instant::now() + self.inner.config.screen_share_grace;
            state.share_grace = Some(state.share_grace.map_or(until, |grace| grace.max(until)));
            schedule_share_grace_end(Arc::downgrade(&self.inner), until);
        }
        // The connection is gone; a failed outcome fence was already logged.
        let _ = self.finish_desktop_transition(state, was_held).await;
    }

    /// Ends the grace that ran out at `until`, unless a report already ended
    /// it or a later disconnect extended it.
    async fn end_share_grace(&self, until: Instant) {
        let mut state = self.inner.state.lock().await;
        if state.share_grace != Some(until) {
            return;
        }
        let was_held = state.desktop_hold();
        state.share_grace = None;
        // Leaving a hold fences nothing; there is no connection to answer.
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
    /// Any helper reports an inactive or sleeping session, or a screen share
    /// while settings hold suggestions during one.
    pub(super) fn desktop_hold(&self) -> bool {
        self.desktop.values().any(|report| report.session.holds())
            || self.screen_share() == ScreenShare::Held
    }

    /// Whether any helper reports a live screen share, or one is in its
    /// grace after its helper left, and what settings do with it. A settings
    /// change already closes every session and advances the epoch, so
    /// switching the mode needs no transition of its own.
    fn screen_share(&self) -> ScreenShare {
        if self.share_grace.is_none() && !self.desktop.values().any(|report| report.screen_shared) {
            return ScreenShare::None;
        }
        match self
            .settings
            .as_ref()
            .map_or_else(ScreenShareMode::default, |settings| settings.screen_share)
        {
            ScreenShareMode::Hold => ScreenShare::Held,
            ScreenShareMode::Suggest => ScreenShare::Suggesting,
        }
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
        } else if self.desktop.values().any(|report| report.session.holds()) {
            DesktopSession::Inactive
        } else {
            DesktopSession::Active
        };
        DesktopStatusPayload {
            observed: !self.desktop.is_empty(),
            session,
            low_power: self.low_power(),
            automatic: self.automatic_scheduling(),
            screen_share: self.screen_share(),
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

/// Wakes when a share's grace ends without keeping a stopped broker alive.
fn schedule_share_grace_end(inner: Weak<BrokerInner>, until: Instant) {
    let Some(shutdown) = inner.upgrade().map(|inner| inner.shutdown.clone()) else {
        return;
    };
    tokio::spawn(async move {
        tokio::select! {
            () = tokio::time::sleep_until(tokio::time::Instant::from_std(until)) => {}
            () = shutdown.cancelled() => return,
        }
        if let Some(inner) = inner.upgrade() {
            Broker { inner }.end_share_grace(until).await;
        }
    });
}
