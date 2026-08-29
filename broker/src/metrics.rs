use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default)]
pub struct Metrics {
    context_updates: AtomicU64,
    provider_calls: AtomicU64,
    provider_input_bytes: AtomicU64,
    provider_output_bytes: AtomicU64,
    cancellations: AtomicU64,
    stale_results: AtomicU64,
    suggestions_shown: AtomicU64,
    suggestions_expired: AtomicU64,
    denied: AtomicU64,
    manual_required: AtomicU64,
    dismissals: AtomicU64,
    commits_prepared: AtomicU64,
    commits_applied: AtomicU64,
    commit_failures: AtomicU64,
    provider_errors: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsSnapshot {
    pub context_updates: u64,
    pub provider_calls: u64,
    pub provider_input_bytes: u64,
    pub provider_output_bytes: u64,
    pub cancellations: u64,
    pub stale_results: u64,
    pub suggestions_shown: u64,
    pub suggestions_expired: u64,
    pub denied: u64,
    pub manual_required: u64,
    pub dismissals: u64,
    pub commits_prepared: u64,
    pub commits_applied: u64,
    pub commit_failures: u64,
    pub provider_errors: u64,
}

impl Metrics {
    pub fn record_context_update(&self) {
        self.context_updates.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_provider_call(&self, input_bytes: usize) {
        self.provider_calls.fetch_add(1, Ordering::Relaxed);
        self.provider_input_bytes
            .fetch_add(saturating_u64(input_bytes), Ordering::Relaxed);
    }

    pub fn record_provider_output(&self, output_bytes: usize) {
        self.provider_output_bytes
            .fetch_add(saturating_u64(output_bytes), Ordering::Relaxed);
    }

    pub fn record_cancellation(&self) {
        self.cancellations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_stale_result(&self) {
        self.stale_results.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_suggestion_shown(&self) {
        self.suggestions_shown.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_suggestion_expired(&self) {
        self.suggestions_expired.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_denied(&self) {
        self.denied.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_manual_required(&self) {
        self.manual_required.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_dismissal(&self) {
        self.dismissals.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_commit_prepared(&self) {
        self.commits_prepared.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_commit_applied(&self) {
        self.commits_applied.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_commit_failure(&self) {
        self.commit_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_provider_error(&self) {
        self.provider_errors.fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            context_updates: self.context_updates.load(Ordering::Relaxed),
            provider_calls: self.provider_calls.load(Ordering::Relaxed),
            provider_input_bytes: self.provider_input_bytes.load(Ordering::Relaxed),
            provider_output_bytes: self.provider_output_bytes.load(Ordering::Relaxed),
            cancellations: self.cancellations.load(Ordering::Relaxed),
            stale_results: self.stale_results.load(Ordering::Relaxed),
            suggestions_shown: self.suggestions_shown.load(Ordering::Relaxed),
            suggestions_expired: self.suggestions_expired.load(Ordering::Relaxed),
            denied: self.denied.load(Ordering::Relaxed),
            manual_required: self.manual_required.load(Ordering::Relaxed),
            dismissals: self.dismissals.load(Ordering::Relaxed),
            commits_prepared: self.commits_prepared.load(Ordering::Relaxed),
            commits_applied: self.commits_applied.load(Ordering::Relaxed),
            commit_failures: self.commit_failures.load(Ordering::Relaxed),
            provider_errors: self.provider_errors.load(Ordering::Relaxed),
        }
    }
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
