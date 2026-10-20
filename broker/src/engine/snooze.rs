//! Snooze: the user turns Badi off for a few minutes in the app or site of
//! the focused field, like Cotypist's snooze shortcut.
//!
//! A snooze is runtime state, never a setting: it is keyed by the exact
//! settings identity of the focused target (an app id or a site origin) and
//! lost when the broker restarts. It only withholds (ADR 0005). Starting one
//! advances the authority epoch and revokes the context, pending inference,
//! visible suggestion and pending commit of every session on that identity;
//! policy then answers `snoozed` there. When it runs out, the epoch advances
//! again so adapters resolve their policy afresh; `snooze_end` (`badi
//! resume`) ends every snooze early.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use super::{Broker, BrokerInner, BrokerState, duration_millis};
use crate::protocol::{PolicyResolutionReason, ReasonCode, SnoozeStatus, TargetDescriptor};
use crate::settings::StableIdentity;

impl Broker {
    /// Snoozes the app or site of the one focused field Badi follows.
    /// Refuses with `paused` while every grant is already withheld,
    /// `no_context` without a focused field and `ambiguous_session` when
    /// focused fields belong to different apps or sites.
    pub async fn snooze_focused(&self, duration: Duration) -> Result<SnoozeStatus, ReasonCode> {
        let mut state = self.inner.state.lock().await;
        if state.effective_paused() {
            return Err(ReasonCode::Paused);
        }
        let subject = state.focused_subject()?;
        let until = Instant::now() + duration;
        state.snoozes.insert(subject.clone(), until);
        state.advance_authority_epoch();
        for session in state.sessions.values_mut() {
            if StableIdentity::from_target(&session.target.target).is_ok_and(|id| id == subject) {
                session.revoke_context_authority(&self.inner.metrics, ReasonCode::Paused);
            }
        }
        self.publish_authority_change(state);
        schedule_snooze_end(Arc::downgrade(&self.inner), until);
        Ok(SnoozeStatus {
            subject,
            remaining_ms: duration_millis(duration),
        })
    }

    /// Ends every snooze now; returns how many were still running.
    pub async fn end_snoozes(&self) -> usize {
        let mut state = self.inner.state.lock().await;
        let now = Instant::now();
        let running = state.snoozes.values().filter(|until| **until > now).count();
        state.snoozes.clear();
        if running > 0 {
            state.advance_authority_epoch();
            self.publish_authority_change(state);
        }
        running
    }

    /// Drops the snoozes that ran out and, if any did, tells every adapter
    /// to resolve its policy again.
    async fn expire_snoozes(&self) {
        let mut state = self.inner.state.lock().await;
        let now = Instant::now();
        let before = state.snoozes.len();
        state.snoozes.retain(|_, until| *until > now);
        if state.snoozes.len() != before {
            state.advance_authority_epoch();
            self.publish_authority_change(state);
        }
    }
}

/// Wakes at the snooze's end without keeping a stopped broker alive. A
/// replaced or ended snooze makes the wake-up find nothing to expire.
fn schedule_snooze_end(inner: Weak<BrokerInner>, until: Instant) {
    let Some(shutdown) = inner.upgrade().map(|inner| inner.shutdown.clone()) else {
        return;
    };
    tokio::spawn(async move {
        tokio::select! {
            () = tokio::time::sleep_until(tokio::time::Instant::from_std(until)) => {}
            () = shutdown.cancelled() => return,
        }
        if let Some(inner) = inner.upgrade() {
            Broker { inner }.expire_snoozes().await;
        }
    });
}

impl BrokerState {
    /// The identity of the focused field: every focused session with context
    /// must belong to the same app or site.
    fn focused_subject(&self) -> Result<StableIdentity, ReasonCode> {
        let mut subjects = self
            .sessions
            .values()
            .filter(|session| {
                session
                    .context
                    .as_ref()
                    .is_some_and(|context| context.field.focused)
            })
            .filter_map(|session| StableIdentity::from_target(&session.target.target).ok());
        let subject = subjects.next().ok_or(ReasonCode::NoContext)?;
        if subjects.any(|other| other != subject) {
            return Err(ReasonCode::AmbiguousSession);
        }
        Ok(subject)
    }

    pub(super) fn snoozed(&self, target: &TargetDescriptor) -> bool {
        !self.snoozes.is_empty()
            && StableIdentity::from_target(target)
                .ok()
                .and_then(|subject| self.snoozes.get(&subject))
                .is_some_and(|until| *until > Instant::now())
    }

    /// Why this target's grants are withheld right now, short of a pause or
    /// a settings denial: a snooze, or an editor holding its terminal.
    pub(super) fn withheld_reason(
        &self,
        target: &TargetDescriptor,
    ) -> Option<PolicyResolutionReason> {
        if self.snoozed(target) {
            Some(PolicyResolutionReason::Snoozed)
        } else if self.editor_withholds(target) {
            Some(PolicyResolutionReason::EditorFocused)
        } else {
            None
        }
    }

    /// Every running snooze, in identity order.
    pub(super) fn snooze_statuses(&self) -> Vec<SnoozeStatus> {
        let now = Instant::now();
        let mut statuses = self
            .snoozes
            .iter()
            .filter(|(_, until)| **until > now)
            .map(|(subject, until)| SnoozeStatus {
                subject: subject.clone(),
                remaining_ms: duration_millis(until.duration_since(now)),
            })
            .collect::<Vec<_>>();
        statuses.sort_by(|left, right| left.subject.cmp(&right.subject));
        statuses
    }
}
