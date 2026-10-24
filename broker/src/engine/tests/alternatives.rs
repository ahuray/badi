//! Alternatives on request: each press shows the model's next-best
//! continuation with a new first word, then cycles through those shown.

use super::*;
use crate::protocol::SuggestionShowPayload;
use crate::provider::DeterministicPhraseProvider;

async fn alternative_to(
    broker: &Broker,
    session_id: SessionId,
    events: &mut mpsc::Receiver<BrokerEvent>,
    shown: &SuggestionShowPayload,
) -> Option<SuggestionShowPayload> {
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Alternatives,
                fingerprint: shown.fingerprint.clone(),
                suggestion_id: Some(shown.suggestion_id.clone()),
            },
            Some("fcitx.alternative.1".to_owned()),
        )
        .await
        .expect("alternatives");
    loop {
        match timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionShow { payload, .. })) => break Some(payload),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break None,
        }
    }
}

#[tokio::test]
async fn presses_show_new_first_words_then_the_shown_ones_again() {
    let broker = Broker::new(
        std::sync::Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    let (session, mut events) = open_observed_session(&broker, "field-epoch-1").await;
    let first = shown_after(&broker, session, &mut events, "thank you")
        .await
        .expect("suggestion");
    assert_eq!(first.text, " for your time");
    let mut texts = vec![first.text.clone()];
    let mut current = first;
    for _ in 0..4 {
        current = alternative_to(&broker, session, &mut events, &current)
            .await
            .expect("an alternative or a shown one again");
        texts.push(current.text.clone());
    }
    assert_eq!(
        texts,
        [
            " for your time",
            " so much",
            " again",
            " for your time",
            " so much"
        ],
        "without a fourth first word the first returns, then presses cycle"
    );
    assert_eq!(
        current.accept_word, " so",
        "Tab takes an alternative's word as usual"
    );
}

#[tokio::test]
async fn a_correction_or_another_suggestion_has_no_alternative() {
    let broker = Broker::new(
        std::sync::Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    let (session, mut events) = open_observed_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, session, &mut events, "thank you")
        .await
        .expect("suggestion");
    let error = broker
        .session_control(
            coordinates(session, 1),
            SessionControlRequestPayload {
                action: ControlAction::Alternatives,
                fingerprint: shown.fingerprint.clone(),
                suggestion_id: Some("another-suggestion".to_owned()),
            },
            None,
        )
        .await
        .expect_err("not the shown suggestion");
    assert!(matches!(error, BrokerError::Stale));
    let error = broker
        .session_control(
            coordinates(session, 1),
            SessionControlRequestPayload {
                action: ControlAction::Alternatives,
                fingerprint: shown.fingerprint,
                suggestion_id: None,
            },
            None,
        )
        .await
        .expect_err("names no suggestion");
    assert!(matches!(
        error,
        BrokerError::Protocol(_) | BrokerError::InvalidPayload
    ));
}
