//! Type-through carries suggestion text, never authority (ADR 0004).

use super::*;
use crate::metrics::CarryMiss;

/// An observed browser field (IME-parity): Fcitx on protocol v2, a browser
/// target scoped by its exact site origin, a new target id per edit.
async fn open_browser_session(
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
                    kind: TargetKind::Browser,
                    app_id: "brave-browser".to_owned(),
                    target_id: target_id.to_owned(),
                    origin: Some(Origin {
                        scheme: OriginScheme::Https,
                        host: "example.org".to_owned(),
                        port: None,
                    }),
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                adapter_kind: AdapterKind::Fcitx,
                capabilities: vec![
                    Capability::Context,
                    Capability::Suggestion,
                    Capability::CommitDispatchedUnverified,
                ],
            },
            event_sink(sink),
        )
        .await
        .expect("open browser session");
    (session_id, receiver)
}

/// `shown_after` with one context field adjusted before publishing.
async fn shown_after_tuned(
    broker: &Broker,
    session_id: SessionId,
    events: &mut mpsc::Receiver<BrokerEvent>,
    before: &str,
    tune: impl FnOnce(&mut ContextChangedPayload),
) -> Option<crate::protocol::SuggestionShowPayload> {
    let mut update = context(1, FieldPurpose::Normal);
    update.before = before.to_owned();
    tune(&mut update);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
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
                        word: None,
                    },
                    None,
                )
                .await
                .expect("control");
        } else {
            assert!(broker.set_paused(true).await);
            assert!(!broker.set_paused(false).await);
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
                word: None,
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

#[tokio::test]
async fn a_single_typed_space_carries_in_a_browser_field_and_is_counted() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut first_events) = open_browser_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut first_events, "Thank you")
        .await
        .expect("generated suggestion");
    assert_eq!(shown.text, " revision Thank you");
    broker
        .close_session(coordinates(first, 1))
        .await
        .expect("close");

    let (second, mut second_events) = open_browser_session(&broker, "field-epoch-2").await;
    let carried = shown_after(&broker, second, &mut second_events, "Thank you ")
        .await
        .expect("carried remainder");
    assert_eq!(carried.text, "revision Thank you");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    let type_through = broker
        .inner
        .metrics
        .snapshot()
        .type_through
        .expect("type-through breakdown");
    assert_eq!(type_through.carried, 1);
    assert_eq!(type_through.misses(), 0);
}

#[tokio::test]
async fn a_declined_carry_records_its_gate_when_generation_runs() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut first_events) = open_browser_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut first_events, "Thank you")
        .await
        .expect("generated suggestion");
    assert_eq!(shown.text, " revision Thank you");
    broker
        .close_session(coordinates(first, 1))
        .await
        .expect("close");

    // The same typed space in the same site, but the new context names a
    // different language: the carry declines and the model answers instead.
    let (second, mut second_events) = open_browser_session(&broker, "field-epoch-2").await;
    let _ = shown_after_tuned(
        &broker,
        second,
        &mut second_events,
        "Thank you ",
        |update| {
            update.language = Some("de".to_owned());
        },
    )
    .await;
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
    let type_through = broker
        .inner
        .metrics
        .snapshot()
        .type_through
        .expect("type-through breakdown");
    assert_eq!(type_through.carried, 0);
    assert_eq!(type_through.count(CarryMiss::LanguageChanged), 1);
    assert_eq!(type_through.misses(), 1);
    assert_eq!(type_through.last_miss, Some(CarryMiss::LanguageChanged));
}

#[tokio::test]
async fn a_carried_remainder_survives_its_field_being_admitted_again_unchanged() {
    // Live Chromium: the typed character changes the field's accessibility
    // epoch, so the same context arrives once more under a new session.
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (first, mut first_events) = open_browser_session(&broker, "field-epoch-1").await;
    shown_after(&broker, first, &mut first_events, "Thank you")
        .await
        .expect("generated suggestion");
    let (second, mut second_events) = open_browser_session(&broker, "field-epoch-2").await;
    let carried = shown_after(&broker, second, &mut second_events, "Thank you r")
        .await
        .expect("carried remainder");
    assert_eq!(carried.text, "evision Thank you");

    let (third, mut third_events) = open_browser_session(&broker, "field-epoch-3").await;
    let again = shown_after(&broker, third, &mut third_events, "Thank you r")
        .await
        .expect("the same remainder");
    assert_eq!(again.text, "evision Thank you");
    assert!(again.ttl_ms <= carried.ttl_ms);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    let type_through = broker
        .inner
        .metrics
        .snapshot()
        .type_through
        .expect("type-through breakdown");
    assert_eq!(type_through.carried, 2);
    assert_eq!(type_through.misses(), 0);

    // Asking explicitly for that context still runs the model.
    let (fourth, mut fourth_events) = open_browser_session(&broker, "field-epoch-4").await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "Thank you r".to_owned();
    broker
        .update_context(coordinates(fourth, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(fourth, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: true,
            },
            None,
        )
        .await
        .expect("request");
    loop {
        match timeout(Duration::from_millis(100), fourth_events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionShow { .. }) | None) | Err(_) => break,
            Ok(Some(_)) => {}
        }
    }
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
}

/// Records every suggestion length the engine hands it.
struct LengthProvider {
    lengths: std::sync::Mutex<Vec<crate::settings::SuggestionLength>>,
    calls: AtomicU64,
}

#[async_trait]
impl CompletionProvider for LengthProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    fn set_suggestion_length(&self, length: crate::settings::SuggestionLength) {
        self.lengths.lock().expect("lengths").push(length);
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Some(format!(" revision {}", request.before)))
    }
}

/// Telegram's observed fields allowed, at `length`.
fn telegram_settings(revision: u64, length: crate::settings::SuggestionLength) -> SettingsV2 {
    let mut settings = controlled_settings(revision, true, false);
    settings.suggestion_length = length;
    settings.subjects = vec![SubjectRule {
        identity: StableIdentity::linux_app(
            crate::settings::LinuxAdapter::Fcitx,
            "org.telegram.desktop",
        )
        .expect("Telegram identity"),
        permissions: settings.subjects[0].permissions,
    }];
    settings
}

/// An observed Telegram field on a broker with settings, which also needs
/// the policy capability.
async fn open_telegram_session(
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
                    Capability::Policy,
                    Capability::CommitDispatchedUnverified,
                ],
            },
            event_sink(sink),
        )
        .await
        .expect("open Telegram session");
    (session_id, receiver)
}

#[tokio::test]
async fn a_new_suggestion_length_reaches_the_provider_and_ends_the_carry() {
    use crate::settings::SuggestionLength;
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, telegram_settings(1, SuggestionLength::Short))
        .expect("initial settings");
    let provider = std::sync::Arc::new(LengthProvider {
        lengths: std::sync::Mutex::new(Vec::new()),
        calls: AtomicU64::new(0),
    });
    let broker = Broker::with_control_plane(
        std::sync::Arc::clone(&provider) as std::sync::Arc<dyn CompletionProvider>,
        BrokerConfig::default(),
        control_plane,
    )
    .expect("controlled broker");
    assert_eq!(
        *provider.lengths.lock().expect("lengths"),
        [SuggestionLength::Short],
        "the stored length applies from the start"
    );

    let (first, mut first_events) = open_telegram_session(&broker, "field-epoch-1").await;
    let shown = shown_after(&broker, first, &mut first_events, "Hello")
        .await
        .expect("generated suggestion");
    assert_eq!(shown.text, " revision Hello");
    broker
        .replace_settings(1, telegram_settings(2, SuggestionLength::Normal))
        .await
        .expect("normal suggestions");
    assert_eq!(
        *provider.lengths.lock().expect("lengths"),
        [SuggestionLength::Short, SuggestionLength::Normal]
    );

    // Text shown under the earlier length is never carried past the change:
    // typing its next letters asks the provider at the new length.
    let (second, mut second_events) = open_telegram_session(&broker, "field-epoch-2").await;
    let next = shown_after(&broker, second, &mut second_events, "Hello rev")
        .await
        .expect("generated suggestion");
    assert_eq!(next.text, " revision Hello rev");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
}
