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
                word: None,
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

async fn open_list_session(
    broker: &Broker,
    target_id: &str,
) -> (SessionId, mpsc::Receiver<BrokerEvent>) {
    let session_id = SessionId::new();
    let (sink, receiver) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::DesktopApplication,
                    app_id: "org.telegram.desktop".to_owned(),
                    target_id: target_id.to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                adapter_kind: AdapterKind::Fcitx,
                capabilities: vec![
                    Capability::Context,
                    Capability::Suggestion,
                    Capability::CommitDispatchedUnverified,
                    Capability::AlternativeList,
                ],
            },
            event_sink(sink),
        )
        .await
        .expect("open list session");
    (session_id, receiver)
}

fn alternatives_payload(
    shown: &SuggestionShowPayload,
    word: Option<&str>,
) -> SessionControlRequestPayload {
    SessionControlRequestPayload {
        action: ControlAction::Alternatives,
        fingerprint: shown.fingerprint.clone(),
        suggestion_id: Some(shown.suggestion_id.clone()),
        word: word.map(str::to_owned),
    }
}

#[tokio::test]
async fn a_list_capable_session_gets_words_and_a_pick_shows_its_continuation() {
    let broker = Broker::new(
        std::sync::Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    let (session, mut events) = open_list_session(&broker, "field-epoch-1").await;
    let first = shown_after(&broker, session, &mut events, "thank you")
        .await
        .expect("suggestion");
    assert_eq!(first.text, " for your time");

    // A pick before any list is live names nothing the broker offered.
    let error = broker
        .session_control(
            coordinates(session, 1),
            alternatives_payload(&first, Some("again")),
            None,
        )
        .await
        .expect_err("no list is live");
    assert!(matches!(error, BrokerError::Stale));

    broker
        .session_control(
            coordinates(session, 1),
            alternatives_payload(&first, None),
            None,
        )
        .await
        .expect("list request");
    let list = loop {
        match timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionList { payload, .. })) => break payload,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => panic!("expected an alternative list"),
        }
    };
    assert_eq!(list.suggestion_id, first.suggestion_id);
    assert_eq!(list.words, ["so", "again"]);
    list.validate().expect("a valid list payload");

    broker
        .session_control(
            coordinates(session, 1),
            alternatives_payload(&first, Some("again")),
            None,
        )
        .await
        .expect("pick");
    let picked = loop {
        match timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionShow { payload, .. })) => break payload,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => panic!("expected the picked continuation"),
        }
    };
    assert_eq!(picked.text, " again");

    // The list is spent once a word is picked.
    let error = broker
        .session_control(
            coordinates(session, 1),
            alternatives_payload(&picked, Some("so")),
            None,
        )
        .await
        .expect_err("the list is spent");
    assert!(matches!(error, BrokerError::Stale));
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
async fn an_alternative_to_a_carried_remainder_asks_the_model() {
    let broker = Broker::new(
        std::sync::Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    let (first, mut events) = open_observed_session(&broker, "field-epoch-1").await;
    shown_after(&broker, first, &mut events, "thank you")
        .await
        .expect("suggestion");
    let (typed, mut events) = open_observed_session(&broker, "field-epoch-2").await;
    let carried = shown_after(&broker, typed, &mut events, "thank you f")
        .await
        .expect("the remainder");
    assert_eq!(carried.text, "or your time");
    let calls = broker.metrics().snapshot().provider_calls;
    let _ = alternative_to(&broker, typed, &mut events, &carried).await;
    assert_eq!(
        broker.metrics().snapshot().provider_calls,
        calls + 1,
        "the carry does not answer for the model"
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
                word: None,
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
                word: None,
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
