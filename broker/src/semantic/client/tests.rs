use super::*;

#[test]
fn stream_counts_come_from_per_token_timings() {
    let started = Instant::now();
    let mut accumulator = StreamAccumulator::default();
    assert_eq!(accumulator.counts(), (None, None, 0));
    accumulator
        .accept_event(
            r#"{"index":0,"content":" for","stop":false,"timings":{"prompt_n":41,"cache_n":33,"predicted_n":1}}"#,
            started,
        )
        .expect("token event");
    assert_eq!(accumulator.counts(), (Some(41), Some(33), 1));
    assert!(accumulator.ttft.is_some());
    accumulator
        .accept_event(
            r#"{"index":0,"content":" the","stop":false,"timings":{"prompt_n":41,"cache_n":33,"predicted_n":2}}"#,
            started,
        )
        .expect("token event");
    // Without timings the generated count falls back to token events.
    accumulator.timings = None;
    assert_eq!(accumulator.counts(), (None, None, 2));

    // Malformed optional statistics stay unknown instead of rejecting
    // an otherwise valid token event.
    let mut malformed = StreamAccumulator::default();
    malformed
        .accept_event(
            r#"{"index":0,"content":" for","stop":false,"timings":[]}"#,
            started,
        )
        .expect("token event with malformed timings");
    malformed
        .accept_event(
            r#"{"index":0,"content":" the","stop":false,"timings":{"prompt_n":-1,"cache_n":"33","predicted_n":1.5}}"#,
            started,
        )
        .expect("token event with malformed timing fields");
    assert_eq!(malformed.counts(), (None, None, 2));
}

#[test]
fn text_after_the_caret_is_eligible_unless_the_caret_splits_a_word() {
    let request = |before: &str, after: &str| ProviderRequest {
        before: before.to_owned(),
        after: after.to_owned(),
        language: Some("en".to_owned()),
        vocabulary: None,
    };
    for (before, after, expected) in [
        ("Please find the", "", InputEligibility::Eligible),
        (
            "Please find the",
            " report today.",
            InputEligibility::Eligible,
        ),
        (
            "Please find the ",
            "report today.",
            InputEligibility::Eligible,
        ),
        ("Please find the upd", ".", InputEligibility::Eligible),
        (
            "Please find the rep",
            "ort today.",
            InputEligibility::Abstain,
        ),
    ] {
        assert_eq!(
            validate_request(&request(before, after)).expect("valid request"),
            expected,
            "{before:?} | {after:?}"
        );
    }
}

#[tokio::test]
async fn cancelled_and_expired_writing_request_remains_cancelled() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("listener");
    let client = SemanticClient::new(
        SemanticClientConfig::new(
            listener.local_addr().expect("endpoint"),
            "budget-priority-fixture",
            "public-fixture-token",
        )
        .expect("config"),
    )
    .expect("client");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let expired = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .expect("two seconds before test request");
    let budget = RequestTrigger::Explicit.writing_budget();
    assert!(matches!(
        client
            .complete_inner(
                Vec::new(),
                expired,
                budget,
                cancellation,
                StreamShape::default()
            )
            .await,
        Err(ClientError::Cancelled)
    ));
    let abstention = client
        .complete_inner(
            Vec::new(),
            expired,
            budget,
            CancellationToken::new(),
            StreamShape::default(),
        )
        .await
        .expect("expired budget");
    assert_eq!(
        abstention.disposition(),
        CompletionDisposition::ModelAbstained
    );
    assert_eq!(
        abstention.no_suggestion_reason(),
        Some(NoSuggestionReason::BudgetPrefill)
    );
    assert_eq!(abstention.request_body_bytes(), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .is_err()
    );
}
