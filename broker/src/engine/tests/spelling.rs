//! Spelling replacement: negotiated authority, exact suffix binding and stale rejection.

use super::*;

#[tokio::test]
async fn spelling_replacement_binds_original_suffix_through_commit() {
    for (original, corrected) in [("teh", "the"), ("teh ", "the ")] {
        for action in [ControlAction::AcceptWord, ControlAction::AcceptAll] {
            assert_spelling_replacement_commits(original, corrected, action).await;
        }
    }
}

async fn assert_spelling_replacement_commits(
    original: &'static str,
    corrected: &'static str,
    action: ControlAction,
) {
    let (broker, id, mut events) = setup_with_capabilities(
        std::sync::Arc::new(SpellingProvider(original, corrected)),
        BrokerConfig::default(),
        vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
            Capability::TextReplacement,
        ],
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = format!("This is {original}");
    update.selection.anchor = u64::try_from(update.before.len()).expect("small fixture");
    update.selection.head = update.selection.anchor;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionShow { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("expected spelling preview")
    };
    assert_eq!(payload.replace_before.as_deref(), Some(original));
    assert_eq!(payload.text, corrected);
    assert_eq!(payload.accept_word, corrected);
    broker
        .session_control(
            coordinates(id, 1),
            SessionControlRequestPayload {
                action,
                fingerprint: update.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
            },
            None,
        )
        .await
        .expect("accept");
    let BrokerEvent::CommitPrepare { payload, .. } = events.recv().await.expect("commit") else {
        panic!("expected commit")
    };
    assert_eq!(payload.replace_before.as_deref(), Some(original));
    assert_eq!(payload.text, corrected);
}

#[tokio::test]
async fn provider_cannot_send_replacements_to_an_old_adapter() {
    let (broker, id, mut events) = setup(
        std::sync::Arc::new(SpellingProvider("teh", "the")),
        BrokerConfig::default(),
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "This is teh".to_owned();
    update.selection.anchor = 11;
    update.selection.head = 11;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionClear { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("expected capability rejection")
    };
    assert_eq!(payload.reason, ReasonCode::InvalidCapability);
}

#[tokio::test]
async fn correction_cannot_change_spacing_or_replace_an_unbound_suffix() {
    for (before, after, original, corrected) in [
        ("This is teh ", "", "teh ", "the"),
        ("This is teh", "", "teh", "the "),
        ("This is teh  ", "", "teh  ", "the  "),
        ("This is teh\t", "", "teh\t", "the\t"),
        ("This is teh ", "", "other ", "their "),
        ("Thisisteh ", "", "teh ", "the "),
        ("This is teh ", "later", "teh ", "the "),
    ] {
        let (broker, id, mut events) = setup_with_capabilities(
            std::sync::Arc::new(SpellingProvider(original, corrected)),
            BrokerConfig::default(),
            vec![
                Capability::Context,
                Capability::Suggestion,
                Capability::CommitApplied,
                Capability::TextReplacement,
            ],
        )
        .await;
        let mut update = context(1, FieldPurpose::Normal);
        update.before = before.to_owned();
        update.after = after.to_owned();
        update.selection.anchor = u64::try_from(before.len()).expect("small fixture");
        update.selection.head = update.selection.anchor;
        broker
            .update_context(coordinates(id, 1), update.clone())
            .await
            .expect("context");
        broker
            .request_suggestion(
                coordinates(id, 1),
                SuggestRequestPayload {
                    fingerprint: update.fingerprint,
                    explicit: false,
                },
                None,
            )
            .await
            .expect("request");
        let BrokerEvent::SuggestionClear { payload, .. } =
            timeout(Duration::from_millis(100), events.recv())
                .await
                .expect("deadline")
                .expect("event")
        else {
            panic!("unbound spelling preview for {before:?}")
        };
        let metrics = broker.metrics().snapshot();
        assert_eq!(metrics.commits_prepared, 0);
        let breakdown = metrics.no_suggestion.expect("breakdown");
        if after == "later" {
            assert_eq!(payload.reason, ReasonCode::NoSuggestion, "{before:?}");
            assert_eq!(breakdown.request_abstained, 1, "{before:?}");
        } else {
            assert_eq!(payload.reason, ReasonCode::InvalidOutput, "{before:?}");
            assert_eq!(breakdown.output_rejected, 1, "{before:?}");
        }
        assert_eq!(breakdown.total(), 1, "{before:?}");
    }
}

#[tokio::test]
async fn spelling_after_space_is_revoked_when_typing_continues() {
    let (broker, id, mut events) = setup_with_capabilities(
        std::sync::Arc::new(SpellingProvider("teh ", "the ")),
        BrokerConfig::default(),
        vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
            Capability::TextReplacement,
        ],
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "This is teh ".to_owned();
    update.selection.anchor = 12;
    update.selection.head = 12;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let BrokerEvent::SuggestionShow { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("spelling preview")
    };
    let mut next = context(2, FieldPurpose::Normal);
    next.before = "This is teh n".to_owned();
    next.selection.anchor = 13;
    next.selection.head = 13;
    broker
        .update_context(coordinates(id, 2), next)
        .await
        .expect("new context");
    assert!(
        broker
            .session_control(
                coordinates(id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptWord,
                    fingerprint: update.fingerprint,
                    suggestion_id: Some(payload.suggestion_id),
                },
                None,
            )
            .await
            .is_err()
    );
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, BrokerEvent::CommitPrepare { .. }));
    }
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

struct ReplacementAuthorityProbe(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[async_trait]
impl CompletionProvider for ReplacementAuthorityProbe {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }
    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }
    async fn propose(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
        allow_replacement: bool,
    ) -> Result<Option<crate::provider::WritingProposal>, ProviderError> {
        self.0
            .store(allow_replacement, std::sync::atomic::Ordering::SeqCst);
        // Deliberately ignore the false flag to exercise the broker's
        // independent output gate as well as provider configuration.
        Ok(Some(crate::provider::WritingProposal {
            text: "the ".to_owned(),
            replace_before: Some("teh ".to_owned()),
        }))
    }
}

#[tokio::test]
async fn manual_unknown_identity_never_receives_spelling_authority() {
    let allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (broker, id, mut events) = setup_with_capabilities(
        std::sync::Arc::new(ReplacementAuthorityProbe(std::sync::Arc::clone(&allowed))),
        BrokerConfig::default(),
        vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::TextReplacement,
        ],
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "This is teh ".to_owned();
    update.selection.anchor = 12;
    update.selection.head = 12;
    update.field.identity_known = false;
    update.explicit = true;
    broker
        .update_context(coordinates(id, 1), update.clone())
        .await
        .expect("explicit context");
    broker
        .request_suggestion(
            coordinates(id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: true,
            },
            None,
        )
        .await
        .expect("explicit request");
    let BrokerEvent::SuggestionClear { payload, .. } =
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("replacement must be rejected")
    };
    assert_eq!(payload.reason, ReasonCode::InvalidCapability);
    assert!(!allowed.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

#[tokio::test]
async fn ime_parity_apps_do_not_receive_spelling_authority() {
    for app_id in ["chromium", "code", "libreoffice", "zen"] {
        let allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let broker = Broker::new(
            std::sync::Arc::new(ReplacementAuthorityProbe(std::sync::Arc::clone(&allowed))),
            BrokerConfig::default(),
        );
        let session_id = SessionId::new();
        let (sink, mut events) = mpsc::channel(32);
        broker
            .open_session(
                coordinates(session_id, 0),
                SessionOpenPayload {
                    target: TargetDescriptor {
                        kind: if app_id == "libreoffice" {
                            TargetKind::DesktopApplication
                        } else {
                            TargetKind::Browser
                        },
                        app_id: app_id.to_owned(),
                        target_id: "field-1".to_owned(),
                        origin: None,
                    },
                    activation: Activation::Always,
                },
                SessionAuthority {
                    protocol_version: 1,
                    adapter_kind: AdapterKind::Fcitx,
                    capabilities: vec![
                        Capability::Context,
                        Capability::Suggestion,
                        Capability::TextReplacement,
                    ],
                },
                event_sink(sink),
            )
            .await
            .expect("open session");
        let mut update = context(1, FieldPurpose::Normal);
        update.before = "This is teh ".to_owned();
        update.selection.anchor = 12;
        update.selection.head = 12;
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
        let BrokerEvent::SuggestionClear { payload, .. } =
            timeout(Duration::from_millis(100), events.recv())
                .await
                .expect("deadline")
                .expect("event")
        else {
            panic!("{app_id} received a spelling suggestion")
        };
        assert_eq!(payload.reason, ReasonCode::InvalidCapability, "{app_id}");
        assert!(
            !allowed.load(std::sync::atomic::Ordering::SeqCst),
            "{app_id} was offered replacement"
        );
    }
}

/// The Writer extension's session, which shows corrections only.
async fn open_writer_session(
    provider: std::sync::Arc<dyn CompletionProvider>,
) -> (Broker, SessionId, mpsc::Receiver<BrokerEvent>) {
    let broker = Broker::new(provider, BrokerConfig::default());
    let session_id = SessionId::new();
    let (sink, events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Libreoffice,
                    app_id: "libreoffice-writer".to_owned(),
                    target_id: "writer-1".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 2,
                adapter_kind: AdapterKind::Libreoffice,
                capabilities: vec![
                    Capability::Context,
                    Capability::Suggestion,
                    Capability::CommitApplied,
                    Capability::TextReplacement,
                ],
            },
            event_sink(sink),
        )
        .await
        .expect("open writer session");
    (broker, session_id, events)
}

async fn writer_request(
    broker: &Broker,
    session_id: SessionId,
    revision: u64,
    before: &str,
    events: &mut mpsc::Receiver<BrokerEvent>,
) -> BrokerEvent {
    let mut update = context(revision, FieldPurpose::Normal);
    update.before = before.to_owned();
    update.selection.anchor = u64::try_from(before.len()).expect("small fixture");
    update.selection.head = update.selection.anchor;
    broker
        .update_context(coordinates(session_id, revision), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, revision),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    timeout(Duration::from_millis(200), events.recv())
        .await
        .expect("deadline")
        .expect("event")
}

#[tokio::test]
async fn the_writer_extension_gets_corrections_and_never_a_continuation() {
    let counting = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, id, mut events) = open_writer_session(counting.clone()).await;
    // A known word: no provider call at all.
    let known = writer_request(&broker, id, 1, "This is fine ", &mut events).await;
    assert!(
        matches!(known, BrokerEvent::SuggestionClear { .. }),
        "{known:?}"
    );
    assert_eq!(counting.calls.load(Ordering::Relaxed), 0);
    // An unknown word whose provider answers with a continuation: withheld.
    let continued = writer_request(&broker, id, 2, "This is teh ", &mut events).await;
    assert!(
        matches!(continued, BrokerEvent::SuggestionClear { .. }),
        "{continued:?}"
    );
    assert_eq!(counting.calls.load(Ordering::Relaxed), 1);

    let (broker, id, mut events) =
        open_writer_session(std::sync::Arc::new(SpellingProvider("teh ", "the "))).await;
    let BrokerEvent::SuggestionShow { payload, .. } =
        writer_request(&broker, id, 1, "This is teh ", &mut events).await
    else {
        panic!("expected a correction")
    };
    assert_eq!(payload.replace_before.as_deref(), Some("teh "));
}
