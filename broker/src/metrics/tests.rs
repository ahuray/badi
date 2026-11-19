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
        66
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
fn typo_suppression_round_trips_and_older_breakdowns_still_decode() {
    let metrics = Metrics::default();
    metrics.record_no_suggestion(NoSuggestionReason::TypoSuspected);
    let mut value = serde_json::to_value(metrics.snapshot()).expect("metrics JSON");
    let decoded: MetricsSnapshot = serde_json::from_value(value.clone()).expect("new broker");
    let counts = decoded.no_suggestion.expect("breakdown");
    assert_eq!(counts.typo_suspected, 1);
    assert_eq!(counts.total(), 1);
    assert_eq!(counts.last, Some(NoSuggestionReason::TypoSuspected));
    let breakdown = value["no_suggestion"].as_object_mut().expect("breakdown");
    breakdown.remove("typo_suspected");
    breakdown.remove("last");
    let older: MetricsSnapshot = serde_json::from_value(value).expect("older broker");
    assert_eq!(older.no_suggestion.expect("breakdown").typo_suspected, 0);
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
