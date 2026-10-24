use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Content-free class of a provider call that ended without a displayed
/// suggestion. Each such call is counted under exactly one class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoSuggestionReason {
    /// Missing or unsupported language, text after the caret, an empty prefix,
    /// or a Persian joiner not yet between two letters. No runtime request.
    RequestAbstained,
    /// The writing budget expired before response headers (prompt prefill),
    /// including when an earlier spelling attempt had already spent it.
    BudgetPrefill,
    /// The writing budget expired while streaming, before a complete word.
    BudgetStream,
    /// The model finished without a complete word.
    ModelAbstained,
    /// Output failed language, lexicon, number, shape, safety or authority
    /// validation.
    OutputRejected,
    /// Cancelled, superseded, paused or no longer current before display.
    Stale,
    /// The broker generation or provider deadline expired.
    Timeout,
    /// The runtime failed or returned a malformed response.
    ProviderError,
    /// A token of the suggestion was less likely than the confidence floor
    /// (`writing::CONFIDENCE_FLOOR`, or the app's stricter quiet floor), or
    /// its likelihood was unknown.
    LowConfidence,
    /// The writer turned down the same next word at the same spot in this
    /// app or site before: Escape, or typing other letters twice.
    Repeated,
}

impl NoSuggestionReason {
    const ALL: [Self; 10] = [
        Self::RequestAbstained,
        Self::BudgetPrefill,
        Self::BudgetStream,
        Self::ModelAbstained,
        Self::OutputRejected,
        Self::Stale,
        Self::Timeout,
        Self::ProviderError,
        Self::LowConfidence,
        Self::Repeated,
    ];

    const fn index(self) -> usize {
        match self {
            Self::RequestAbstained => 0,
            Self::BudgetPrefill => 1,
            Self::BudgetStream => 2,
            Self::ModelAbstained => 3,
            Self::OutputRejected => 4,
            Self::Stale => 5,
            Self::Timeout => 6,
            Self::ProviderError => 7,
            Self::LowConfidence => 8,
            Self::Repeated => 9,
        }
    }
}

/// Content-free class of an admitted request that generation answered with a
/// provider call although a type-through carry (ADR 0004) existed. The class
/// names the carry gate that declined; the gates themselves are unchanged.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CarryMiss {
    /// The request is in another app or site than the shown suggestion.
    Scope,
    /// Text after the caret differs from the shown suggestion's context.
    AfterChanged,
    /// The context language differs from the shown suggestion's context.
    LanguageChanged,
    /// Less than the minimum carried reading time remained.
    DeadlineTight,
    /// The new text is not the old text plus a typed proper prefix.
    PrefixMismatch,
    /// The remainder failed sanitization or completion-shape validation.
    ShapeRejected,
}

impl CarryMiss {
    const ALL: [Self; 6] = [
        Self::Scope,
        Self::AfterChanged,
        Self::LanguageChanged,
        Self::DeadlineTight,
        Self::PrefixMismatch,
        Self::ShapeRejected,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Scope => 0,
            Self::AfterChanged => 1,
            Self::LanguageChanged => 2,
            Self::DeadlineTight => 3,
            Self::PrefixMismatch => 4,
            Self::ShapeRejected => 5,
        }
    }
}

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
    words_applied: AtomicU64,
    words_dispatched: AtomicU64,
    commit_failures: AtomicU64,
    provider_errors: AtomicU64,
    no_suggestion: [AtomicU64; 10],
    /// Zero until the first no-suggestion outcome; otherwise its index plus one.
    last_no_suggestion: AtomicU8,
    type_through_carried: AtomicU64,
    carry_miss: [AtomicU64; 6],
    /// Zero until the first carry miss; otherwise its index plus one.
    last_carry_miss: AtomicU8,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NoSuggestionSnapshot {
    pub request_abstained: u64,
    pub budget_prefill: u64,
    pub budget_stream: u64,
    pub model_abstained: u64,
    pub output_rejected: u64,
    pub stale: u64,
    pub timeout: u64,
    pub provider_error: u64,
    /// Absent from brokers that predate the confidence gate.
    #[serde(default)]
    pub low_confidence: u64,
    /// Absent from brokers that predate turned-down words.
    #[serde(default)]
    pub repeated: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<NoSuggestionReason>,
}

impl NoSuggestionSnapshot {
    #[must_use]
    pub const fn count(&self, reason: NoSuggestionReason) -> u64 {
        match reason {
            NoSuggestionReason::RequestAbstained => self.request_abstained,
            NoSuggestionReason::BudgetPrefill => self.budget_prefill,
            NoSuggestionReason::BudgetStream => self.budget_stream,
            NoSuggestionReason::ModelAbstained => self.model_abstained,
            NoSuggestionReason::OutputRejected => self.output_rejected,
            NoSuggestionReason::Stale => self.stale,
            NoSuggestionReason::Timeout => self.timeout,
            NoSuggestionReason::ProviderError => self.provider_error,
            NoSuggestionReason::LowConfidence => self.low_confidence,
            NoSuggestionReason::Repeated => self.repeated,
        }
    }

    #[must_use]
    pub fn total(&self) -> u64 {
        NoSuggestionReason::ALL.iter().fold(0_u64, |total, reason| {
            total.saturating_add(self.count(*reason))
        })
    }
}

/// Type-through outcomes (ADR 0004) since broker start: requests answered
/// from the carry, and admitted requests that reached the provider although
/// a carry existed, by the gate that declined it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypeThroughSnapshot {
    pub carried: u64,
    pub scope: u64,
    pub after_changed: u64,
    pub language_changed: u64,
    pub deadline_tight: u64,
    pub prefix_mismatch: u64,
    pub shape_rejected: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_miss: Option<CarryMiss>,
}

impl TypeThroughSnapshot {
    #[must_use]
    pub const fn count(&self, miss: CarryMiss) -> u64 {
        match miss {
            CarryMiss::Scope => self.scope,
            CarryMiss::AfterChanged => self.after_changed,
            CarryMiss::LanguageChanged => self.language_changed,
            CarryMiss::DeadlineTight => self.deadline_tight,
            CarryMiss::PrefixMismatch => self.prefix_mismatch,
            CarryMiss::ShapeRejected => self.shape_rejected,
        }
    }

    #[must_use]
    pub fn misses(&self) -> u64 {
        CarryMiss::ALL
            .iter()
            .fold(0_u64, |total, miss| total.saturating_add(self.count(*miss)))
    }
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
    /// Absent from protocol v1 and from brokers that predate the breakdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_suggestion: Option<NoSuggestionSnapshot>,
    /// Word completions since broker start; absent from v1 and older brokers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_savings: Option<WordSavings>,
    /// Type-through carry outcomes; absent from protocol v1 and older brokers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_through: Option<TypeThroughSnapshot>,
}

/// An estimate, not a document audit: partial words count and undo is unknown.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WordSavings {
    pub applied: u64,
    pub dispatched_unverified: u64,
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

    pub fn record_word_savings(&self, applied: u64, dispatched_unverified: u64) {
        self.words_applied.fetch_add(applied, Ordering::Relaxed);
        self.words_dispatched
            .fetch_add(dispatched_unverified, Ordering::Relaxed);
    }

    pub fn record_commit_failure(&self) {
        self.commit_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_provider_error(&self) {
        self.provider_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_no_suggestion(&self, reason: NoSuggestionReason) {
        self.no_suggestion[reason.index()].fetch_add(1, Ordering::Relaxed);
        self.last_no_suggestion.store(
            u8::try_from(reason.index() + 1).unwrap_or(0),
            Ordering::Relaxed,
        );
    }

    pub fn record_type_through_carried(&self) {
        self.type_through_carried.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_carry_miss(&self, miss: CarryMiss) {
        self.carry_miss[miss.index()].fetch_add(1, Ordering::Relaxed);
        self.last_carry_miss.store(
            u8::try_from(miss.index() + 1).unwrap_or(0),
            Ordering::Relaxed,
        );
    }

    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        let count =
            |reason: NoSuggestionReason| self.no_suggestion[reason.index()].load(Ordering::Relaxed);
        let last = usize::from(self.last_no_suggestion.load(Ordering::Relaxed))
            .checked_sub(1)
            .and_then(|index| NoSuggestionReason::ALL.get(index).copied());
        let miss_count = |miss: CarryMiss| self.carry_miss[miss.index()].load(Ordering::Relaxed);
        let last_miss = usize::from(self.last_carry_miss.load(Ordering::Relaxed))
            .checked_sub(1)
            .and_then(|index| CarryMiss::ALL.get(index).copied());
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
            word_savings: Some(WordSavings {
                applied: self.words_applied.load(Ordering::Relaxed),
                dispatched_unverified: self.words_dispatched.load(Ordering::Relaxed),
            }),
            no_suggestion: Some(NoSuggestionSnapshot {
                request_abstained: count(NoSuggestionReason::RequestAbstained),
                budget_prefill: count(NoSuggestionReason::BudgetPrefill),
                budget_stream: count(NoSuggestionReason::BudgetStream),
                model_abstained: count(NoSuggestionReason::ModelAbstained),
                output_rejected: count(NoSuggestionReason::OutputRejected),
                stale: count(NoSuggestionReason::Stale),
                timeout: count(NoSuggestionReason::Timeout),
                provider_error: count(NoSuggestionReason::ProviderError),
                low_confidence: count(NoSuggestionReason::LowConfidence),
                repeated: count(NoSuggestionReason::Repeated),
                last,
            }),
            type_through: Some(TypeThroughSnapshot {
                carried: self.type_through_carried.load(Ordering::Relaxed),
                scope: miss_count(CarryMiss::Scope),
                after_changed: miss_count(CarryMiss::AfterChanged),
                language_changed: miss_count(CarryMiss::LanguageChanged),
                deadline_tight: miss_count(CarryMiss::DeadlineTight),
                prefix_mismatch: miss_count(CarryMiss::PrefixMismatch),
                shape_rejected: miss_count(CarryMiss::ShapeRejected),
                last_miss,
            }),
        }
    }
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{CarryMiss, Metrics, MetricsSnapshot, NoSuggestionReason};

    #[test]
    fn each_reason_has_its_own_counter_and_the_last_class_is_retained() {
        let metrics = Metrics::default();
        let empty = metrics.snapshot().no_suggestion.expect("breakdown");
        assert_eq!(empty.total(), 0);
        assert_eq!(empty.last, None);
        for (repeat, reason) in NoSuggestionReason::ALL.into_iter().enumerate() {
            for _ in 0..=repeat {
                metrics.record_no_suggestion(reason);
            }
            let snapshot = metrics.snapshot().no_suggestion.expect("breakdown");
            assert_eq!(
                snapshot.count(reason),
                u64::try_from(repeat + 1).expect("small")
            );
            assert_eq!(snapshot.last, Some(reason));
        }
        assert_eq!(
            metrics.snapshot().no_suggestion.expect("breakdown").total(),
            55
        );
    }

    #[test]
    fn a_breakdown_from_before_the_confidence_gate_still_decodes() {
        let metrics = Metrics::default();
        metrics.record_no_suggestion(NoSuggestionReason::LowConfidence);
        let mut value = serde_json::to_value(metrics.snapshot()).expect("metrics JSON");
        assert_eq!(value["no_suggestion"]["low_confidence"], 1);
        assert_eq!(value["no_suggestion"]["last"], "low_confidence");
        let breakdown = value["no_suggestion"].as_object_mut().expect("breakdown");
        breakdown.remove("low_confidence");
        breakdown.remove("last");
        let decoded: MetricsSnapshot = serde_json::from_value(value).expect("older broker");
        assert_eq!(decoded.no_suggestion.expect("breakdown").low_confidence, 0);
    }

    #[test]
    fn breakdown_serializes_as_text_free_counters_and_stays_optional_for_older_peers() {
        let metrics = Metrics::default();
        metrics.record_no_suggestion(NoSuggestionReason::BudgetPrefill);
        let value = serde_json::to_value(metrics.snapshot()).expect("metrics JSON");
        assert_eq!(value["no_suggestion"]["budget_prefill"], 1);
        assert_eq!(value["no_suggestion"]["last"], "budget_prefill");
        let mut legacy = value.clone();
        for counter in ["no_suggestion", "word_savings", "type_through"] {
            legacy
                .as_object_mut()
                .expect("metrics object")
                .remove(counter);
        }
        let decoded: MetricsSnapshot = serde_json::from_value(legacy).expect("older broker");
        assert_eq!(decoded.no_suggestion, None);
        assert_eq!(decoded.word_savings, None);
        assert_eq!(decoded.type_through, None);
        let mut unknown = value;
        unknown["no_suggestion"]["guessed"] = serde_json::json!(1);
        assert!(serde_json::from_value::<MetricsSnapshot>(unknown).is_err());
    }

    #[test]
    fn each_carry_miss_has_its_own_counter_and_the_last_class_is_retained() {
        let metrics = Metrics::default();
        let empty = metrics.snapshot().type_through.expect("breakdown");
        assert_eq!(empty.carried, 0);
        assert_eq!(empty.misses(), 0);
        assert_eq!(empty.last_miss, None);
        metrics.record_type_through_carried();
        metrics.record_type_through_carried();
        for (repeat, miss) in CarryMiss::ALL.into_iter().enumerate() {
            for _ in 0..=repeat {
                metrics.record_carry_miss(miss);
            }
            let snapshot = metrics.snapshot().type_through.expect("breakdown");
            assert_eq!(
                snapshot.count(miss),
                u64::try_from(repeat + 1).expect("small")
            );
            assert_eq!(snapshot.last_miss, Some(miss));
        }
        let snapshot = metrics.snapshot().type_through.expect("breakdown");
        assert_eq!(snapshot.carried, 2);
        assert_eq!(snapshot.misses(), 21);
        let value = serde_json::to_value(metrics.snapshot()).expect("metrics JSON");
        assert_eq!(value["type_through"]["carried"], 2);
        assert_eq!(value["type_through"]["prefix_mismatch"], 5);
        assert_eq!(value["type_through"]["last_miss"], "shape_rejected");
        let mut unknown = value;
        unknown["type_through"]["guessed"] = serde_json::json!(1);
        assert!(serde_json::from_value::<MetricsSnapshot>(unknown).is_err());
    }
}
