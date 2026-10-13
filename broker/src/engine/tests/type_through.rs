//! Type-through carries suggestion text, never authority (ADR 0004).

use super::*;

#[tokio::test]
async fn typing_through_a_suggestion_carries_its_remainder_into_the_next_field_session() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut first_events) = open_observed_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut first_events, "Hello")
        .await
        .expect("generated suggestion");
    assert_eq!(shown.text, " revision Hello");
    broker
        .close_session(coordinates(first, 1))
        .await
        .expect("close");

    let (second, mut second_events) = open_observed_session(&broker, "field-epoch-2").await;
    let carried = shown_after(&broker, second, &mut second_events, "Hello rev")
        .await
        .expect("carried remainder");
    assert_eq!(carried.text, "ision Hello");
    assert_eq!(carried.accept_word, "ision");
    assert_ne!(carried.suggestion_id, shown.suggestion_id);
    assert!(carried.ttl_ms <= shown.ttl_ms);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);

    // A character that differs from the suggestion asks the model again.
    let (third, mut third_events) = open_observed_session(&broker, "field-epoch-3").await;
    let generated = shown_after(&broker, third, &mut third_events, "Hello rex")
        .await
        .expect("generated suggestion");
    assert_eq!(generated.text, " revision Hello rex");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn escape_whole_acceptance_and_pause_end_type_through() {
    for action in [
        Some(ControlAction::Dismiss),
        Some(ControlAction::AcceptAll),
        None,
    ] {
        let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
        let broker = Broker::new(provider.clone(), BrokerConfig::default());
        let (first, mut events) = open_observed_session(&broker, "field-epoch-1").await;
        let shown = shown_after(&broker, first, &mut events, "Hello")
            .await
            .expect("generated suggestion");
        if let Some(action) = action {
            broker
                .session_control(
                    coordinates(first, 1),
                    SessionControlRequestPayload {
                        action,
                        fingerprint: shown.fingerprint.clone(),
                        suggestion_id: Some(shown.suggestion_id.clone()),
                    },
                    None,
                )
                .await
                .expect("control");
        } else {
            assert!(broker.set_paused(true).await.expect("pause"));
            assert!(!broker.set_paused(false).await.expect("resume"));
        }
        let (second, mut second_events) = open_observed_session(&broker, "field-epoch-2").await;
        let next = shown_after(&broker, second, &mut second_events, "Hello rev")
            .await
            .expect("generated suggestion");
        assert_eq!(next.text, " revision Hello rev", "{action:?}");
        assert_eq!(provider.calls.load(Ordering::Relaxed), 2, "{action:?}");
    }
}

#[tokio::test]
async fn the_remainder_after_a_word_acceptance_carries() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut events) = open_observed_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut events, "Hello")
        .await
        .expect("generated suggestion");
    broker
        .session_control(
            coordinates(first, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptWord,
                fingerprint: shown.fingerprint.clone(),
                suggestion_id: Some(shown.suggestion_id.clone()),
            },
            Some("word".to_owned()),
        )
        .await
        .expect("accept word");
    let Some(BrokerEvent::CommitPrepare { payload, .. }) = events.recv().await else {
        panic!("expected a commit grant");
    };
    assert_eq!(payload.text, " revision");

    let (second, mut second_events) = open_observed_session(&broker, "field-epoch-2").await;
    let carried = shown_after(&broker, second, &mut second_events, "Hello revision")
        .await
        .expect("carried remainder");
    assert_eq!(carried.text, " Hello");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
}
