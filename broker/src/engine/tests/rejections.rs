//! Turned-down next words: Escape, or typing other letters twice, withholds
//! the same automatic next word at the same spot in that app or site until
//! the writer takes it there.

use super::*;

async fn control(
    broker: &Broker,
    session_id: SessionId,
    shown: &crate::protocol::SuggestionShowPayload,
    action: ControlAction,
) {
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action,
                fingerprint: shown.fingerprint.clone(),
                suggestion_id: Some(shown.suggestion_id.clone()),
            },
            None,
        )
        .await
        .expect("control");
}

async fn dismiss(
    broker: &Broker,
    session_id: SessionId,
    shown: &crate::protocol::SuggestionShowPayload,
) {
    control(broker, session_id, shown, ControlAction::Dismiss).await;
}

async fn asked_after(
    broker: &Broker,
    session_id: SessionId,
    events: &mut mpsc::Receiver<BrokerEvent>,
    before: &str,
) -> Option<crate::protocol::SuggestionShowPayload> {
    let mut update = context(1, FieldPurpose::Normal);
    update.before = before.to_owned();
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: true,
            },
            None,
        )
        .await
        .expect("request");
    loop {
        match timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionShow { payload, .. })) => break Some(payload),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break None,
        }
    }
}

fn repeated(broker: &Broker) -> u64 {
    broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown")
        .repeated
}

#[tokio::test]
async fn a_dismissed_next_word_is_not_offered_again_at_that_spot_unless_asked() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut events) = open_observed_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut events, "Hello")
        .await
        .expect("suggestion");
    dismiss(&broker, first, &shown).await;

    let (second, mut events) = open_observed_session(&broker, "field-epoch-2").await;
    assert!(
        shown_after(&broker, second, &mut events, "Hello")
            .await
            .is_none()
    );
    assert_eq!(repeated(&broker), 1);
    assert_eq!(
        provider.calls.load(Ordering::Relaxed),
        2,
        "the model still ran"
    );

    let (third, mut events) = open_observed_session(&broker, "field-epoch-3").await;
    let asked = asked_after(&broker, third, &mut events, "Hello")
        .await
        .expect("an explicit request");
    assert_eq!(asked.text, " revision Hello");
    // Taking it there forgets the turn-down.
    control(&broker, third, &asked, ControlAction::AcceptWord).await;
    let (again, mut events) = open_observed_session(&broker, "field-epoch-3b").await;
    assert!(
        shown_after(&broker, again, &mut events, "Hello")
            .await
            .is_some(),
        "a taken word is offered again"
    );

    let (fourth, mut events) = open_observed_session(&broker, "field-epoch-4").await;
    assert!(
        shown_after(&broker, fourth, &mut events, "Goodbye")
            .await
            .is_some(),
        "another spot still gets the word"
    );
}

#[tokio::test]
async fn leaving_neovim_insert_mode_turns_no_word_down() {
    let broker = Broker::new(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
    );
    for target in ["buffer-epoch-1", "buffer-epoch-2"] {
        let (session, mut events) = open_session_as(&broker, target, AdapterKind::Neovim).await;
        let shown = shown_after(&broker, session, &mut events, "Hello")
            .await
            .expect("shown again after InsertLeave");
        dismiss(&broker, session, &shown).await;
    }
    assert_eq!(repeated(&broker), 0);
}

#[tokio::test]
async fn typing_over_the_same_next_word_twice_withholds_it_there() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    for (round, target) in ["field-epoch-1", "field-epoch-3"].into_iter().enumerate() {
        let (session, mut events) = open_observed_session(&broker, target).await;
        assert!(
            shown_after(&broker, session, &mut events, "Hello")
                .await
                .is_some(),
            "round {round}: shown before the second turn-down"
        );
        broker
            .close_session(coordinates(session, 1))
            .await
            .expect("close");
        // Two keys of other letters at one shown suggestion count once.
        for (key, typed) in ["Hello x", "Hello xy"].into_iter().enumerate() {
            let (other, mut events) =
                open_observed_session(&broker, &format!("{target}-typed-{key}")).await;
            let _ = shown_after(&broker, other, &mut events, typed).await;
            broker
                .close_session(coordinates(other, 1))
                .await
                .expect("close");
        }
    }
    let (session, mut events) = open_observed_session(&broker, "field-epoch-5").await;
    assert!(
        shown_after(&broker, session, &mut events, "Hello")
            .await
            .is_none()
    );
    assert_eq!(repeated(&broker), 1);
}
