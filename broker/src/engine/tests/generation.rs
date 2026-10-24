//! Admission, abstention, cancellation, deadlines and provider capacity.

use super::*;

#[tokio::test]
async fn normal_provider_abstention_clears_without_recording_a_failure() {
    let (broker, id, mut events) = setup(
        std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
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
        panic!("expected clear")
    };
    assert_eq!(payload.reason, ReasonCode::NoSuggestion);
    let metrics = broker.inner.metrics.snapshot();
    assert_eq!(metrics.provider_errors, 0);
    let breakdown = metrics.no_suggestion.expect("breakdown");
    assert_eq!(breakdown.model_abstained, 1);
    assert_eq!(breakdown.total(), 1);
    assert_eq!(breakdown.last, Some(NoSuggestionReason::ModelAbstained));
}

#[tokio::test]
async fn request_side_abstention_is_counted_separately_from_the_model() {
    let (broker, id, mut events) = setup(
        std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    )
    .await;
    let mut update = context(1, FieldPurpose::Normal);
    update.language = Some("ar".to_owned());
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
    assert!(matches!(
        timeout(Duration::from_millis(100), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, .. }))
            if payload.reason == ReasonCode::NoSuggestion
    ));
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.request_abstained, 1);
    assert_eq!(breakdown.total(), 1);
}

#[tokio::test]
async fn a_word_after_the_caret_abstains_before_the_model() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (session_id, mut events) = open_observed_session(&broker, "mid-line-off").await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "Please find the ".to_owned();
    update.after = "report today".to_owned();
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
        panic!("expected an abstention");
    };
    assert_eq!(payload.reason, ReasonCode::NoSuggestion);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.request_abstained, 1);

    let mut punctuated = context(2, FieldPurpose::Normal);
    punctuated.before = "Please review the docum".to_owned();
    punctuated.after = ".".to_owned();
    broker
        .update_context(coordinates(session_id, 2), punctuated.clone())
        .await
        .expect("punctuation context");
    broker
        .request_suggestion(
            coordinates(session_id, 2),
            SuggestRequestPayload {
                fingerprint: punctuated.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("punctuation request");
    timeout(Duration::from_millis(100), async {
        while provider.calls.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("punctuation still reaches the model");
}

#[tokio::test]
async fn superseded_provider_work_is_counted_as_stale_once() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, id, _events) = setup(provider, BrokerConfig::default()).await;
    let update = context(1, FieldPurpose::Normal);
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
    let cancellation = wait_for_provider_token(&provider_view).await;
    broker
        .update_context(coordinates(id, 2), context(2, FieldPurpose::Normal))
        .await
        .expect("newer context");
    assert!(cancellation.is_cancelled());
    timeout(Duration::from_millis(100), async {
        while broker
            .metrics()
            .snapshot()
            .no_suggestion
            .is_none_or(|breakdown| breakdown.stale == 0)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stale class recorded");
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.stale, 1);
    assert_eq!(breakdown.total(), 1);
}

#[tokio::test]
async fn cancelling_a_shown_suggestion_revokes_its_acceptance() {
    let (broker, id, mut events) = setup(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
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
    let BrokerEvent::SuggestionShow { payload: shown, .. } =
        timeout(Duration::from_millis(200), events.recv())
            .await
            .expect("deadline")
            .expect("event")
    else {
        panic!("expected a suggestion")
    };
    assert_eq!(broker.metrics().snapshot().suggestions_shown, 1);
    broker
        .cancel_suggestion(
            coordinates(id, 1),
            crate::protocol::SuggestCancelPayload {
                fingerprint: update.fingerprint.clone(),
                reason: ReasonCode::Stale,
            },
        )
        .await
        .expect("cancel");
    let BrokerEvent::SuggestionClear { payload, .. } = events.recv().await.expect("clear") else {
        panic!("expected the suggestion withdrawn")
    };
    assert_eq!(payload.suggestion_id, Some(shown.suggestion_id.clone()));
    assert!(
        broker
            .session_control(
                coordinates(id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptAll,
                    fingerprint: update.fingerprint,
                    suggestion_id: Some(shown.suggestion_id),
                },
                None
            )
            .await
            .is_err(),
        "a withdrawn suggestion grants no edit"
    );
}

#[tokio::test]
async fn one_hundred_supersessions_never_show_or_commit_stale_text() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::from_millis(8)));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_secs(1),
            ..BrokerConfig::default()
        },
    )
    .await;

    for revision in 1..=101 {
        let update = context(revision, FieldPurpose::Normal);
        broker
            .update_context(coordinates(session_id, revision), update.clone())
            .await
            .expect("context update");
        broker
            .request_suggestion(
                coordinates(session_id, revision),
                SuggestRequestPayload {
                    fingerprint: update.fingerprint,
                    explicit: false,
                },
                Some(format!("request-{revision}")),
            )
            .await
            .expect("suggestion request");
        tokio::task::yield_now().await;
    }

    sleep(Duration::from_millis(30)).await;
    let mut shows = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let BrokerEvent::SuggestionShow { coordinates, .. } = event {
            shows.push(coordinates.revision);
        }
    }
    assert_eq!(shows, vec![101]);

    let fingerprint = context(101, FieldPurpose::Normal).fingerprint;
    let active = broker.active_locator().await.expect("one active session");
    broker
        .session_control(
            coordinates(session_id, 101),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint,
                suggestion_id: active.suggestion_id,
            },
            Some("commit-latest".to_owned()),
        )
        .await
        .expect("latest suggestion remains eligible");
    let (commit, commit_id, commit_text) = timeout(Duration::from_millis(50), async {
        loop {
            if let Some(BrokerEvent::CommitPrepare {
                coordinates,
                payload,
                request_id,
            }) = events.recv().await
            {
                break (coordinates, request_id, payload.text);
            }
        }
    })
    .await
    .expect("commit event");
    assert_eq!(commit.revision, 101);
    assert_eq!(commit_id.as_deref(), Some("commit-latest"));
    assert!(commit_text.contains("101"));
    let metrics = broker.metrics().snapshot();
    // A superseded generation may be cancelled before it produces an
    // output or rejected after a result races cancellation. Both are safe
    // outcomes; neither may render or authorize stale text.
    assert!(metrics.cancellations + metrics.stale_results >= 100);
    assert_eq!(metrics.suggestions_shown, 1);
    assert_eq!(metrics.commits_prepared, 1);
}

#[tokio::test]
async fn provider_timeout_explicitly_cancels_its_token() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_millis(5),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
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
    let cancellation = wait_for_provider_token(&provider_view).await;
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, .. }))
            if payload.reason == ReasonCode::ProviderTimeout
    ));
    assert!(cancellation.is_cancelled());
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.timeout, 1);
    assert_eq!(breakdown.total(), 1);
}

#[tokio::test]
async fn generation_deadline_cancels_late_work() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            generation_timeout: Duration::from_millis(8),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
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
    let cancellation = wait_for_provider_token(&provider_view).await;
    assert!(matches!(
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("timeout event"),
        Some(BrokerEvent::SuggestionClear { payload, .. })
            if payload.reason == ReasonCode::ProviderTimeout
    ));
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn shutdown_cancels_provider_work_and_removes_sessions() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, _events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            ..BrokerConfig::default()
        },
    )
    .await;
    let update = context(1, FieldPurpose::Normal);
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
    let cancellation = wait_for_provider_token(&provider_view).await;

    broker.shutdown().await;

    assert!(cancellation.is_cancelled());
    assert_eq!(broker.session_count().await, 0);
}

#[tokio::test]
async fn provider_admission_is_global_nonblocking_and_released_on_shutdown() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, first_id, _first_events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            provider_concurrency: 1,
            ..BrokerConfig::default()
        },
    )
    .await;
    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(first_id, 1), first.clone())
        .await
        .expect("first context");
    broker
        .request_suggestion(
            coordinates(first_id, 1),
            SuggestRequestPayload {
                fingerprint: first.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("first request");
    let first_cancellation = wait_for_provider_token(&provider_view).await;

    let second_id = SessionId::new();
    let (second_sink, _second_events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(second_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "field-2".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                adapter_kind: AdapterKind::Test,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(second_sink),
        )
        .await
        .expect("second session");
    let second = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(second_id, 1), second.clone())
        .await
        .expect("second context");
    assert!(matches!(
        broker
            .request_suggestion(
                coordinates(second_id, 1),
                SuggestRequestPayload {
                    fingerprint: second.fingerprint,
                    explicit: false,
                },
                None,
            )
            .await,
        Err(BrokerError::ProviderBusy)
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 1);

    broker.shutdown().await;
    assert!(first_cancellation.is_cancelled());
    assert!(broker.inner.provider_admissions.is_closed());
    assert!(broker.inner.shutdown.is_cancelled());
    timeout(Duration::from_millis(50), async {
        while broker.inner.provider_admissions.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider permit released after shutdown cancellation");
}

#[test]
fn provider_admission_configuration_is_clamped_to_hard_bounds() {
    for (configured, expected) in [(0, 1), (usize::MAX, MAX_PROVIDER_CONCURRENCY)] {
        let broker = Broker::new(
            std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
            BrokerConfig {
                provider_concurrency: configured,
                ..BrokerConfig::default()
            },
        );
        assert_eq!(
            broker.inner.provider_admissions.available_permits(),
            expected
        );
    }
}

#[tokio::test]
async fn full_event_queue_fails_connection_closed_without_phantom_suggestion() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider, BrokerConfig::default());
    let session_id = SessionId::new();
    let initial_coordinates = coordinates(session_id, 0);
    let connection_lifetime = CancellationToken::new();
    let (sink, mut events) = mpsc::channel(1);
    sink.try_send(BrokerEvent::SuggestionClear {
        coordinates: initial_coordinates,
        payload: crate::protocol::SuggestionClearPayload {
            fingerprint: "fingerprint_filler".to_owned(),
            suggestion_id: None,
            reason: ReasonCode::Cancelled,
        },
        request_id: None,
    })
    .expect("fill event queue");
    broker
        .open_session(
            initial_coordinates,
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "field-1".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                adapter_kind: AdapterKind::Test,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            BrokerEventSink::new(sink, connection_lifetime.clone()),
        )
        .await
        .expect("open session");
    let update = context(1, FieldPurpose::Normal);
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
    timeout(Duration::from_millis(50), async {
        loop {
            let state = broker.inner.state.lock().await;
            let finished = state
                .sessions
                .get(&session_id)
                .is_some_and(|session| session.cancellation.is_none());
            drop(state);
            if finished {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("generation finished");

    let state = broker.inner.state.lock().await;
    assert!(state.sessions[&session_id].visible.is_none());
    drop(state);
    assert_eq!(broker.metrics().snapshot().suggestions_shown, 0);
    assert!(connection_lifetime.is_cancelled());
    assert!(matches!(
        events.try_recv(),
        Ok(BrokerEvent::SuggestionClear { payload, .. })
            if payload.fingerprint == "fingerprint_filler"
    ));
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn escape_quiet_abstains_automatically_across_edits_until_asked() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    let mut settings = controlled_settings(1, true, false);
    settings.escape_quiet_seconds = 600;
    control_plane
        .replace_settings(0, settings)
        .expect("allow controlled origin with escape quiet");
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::with_control_plane(
        provider.clone(),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
        .expect("allowed session");

    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), first.clone())
        .await
        .expect("allowed context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: first.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("suggestion request");
    let shown = timeout(Duration::from_millis(100), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected suggestion show")
    };
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);

    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Dismiss,
                fingerprint: first.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
            },
            None,
        )
        .await
        .expect("dismiss suggestion");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, .. }) if payload.reason == ReasonCode::Dismissed
    ));

    // An edit ends the adapters' own decline, but the timed quiet holds: the
    // automatic request abstains without a provider call.
    let edited = context(2, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 2), edited.clone())
        .await
        .expect("edited context");
    broker
        .request_suggestion(
            coordinates(session_id, 2),
            SuggestRequestPayload {
                fingerprint: edited.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("automatic request during quiet");
    let cleared = timeout(Duration::from_millis(100), events.recv())
        .await
        .expect("clear timeout")
        .expect("clear event");
    let BrokerEvent::SuggestionClear { payload, .. } = cleared else {
        panic!("expected suggestion clear during quiet")
    };
    assert_eq!(payload.reason, ReasonCode::NoSuggestion);
    assert_eq!(
        provider.calls.load(Ordering::Relaxed),
        1,
        "quiet calls no model"
    );

    // Asking explicitly ends the quiet at once and suggests again.
    broker
        .request_suggestion(
            coordinates(session_id, 2),
            SuggestRequestPayload {
                fingerprint: edited.fingerprint.clone(),
                explicit: true,
            },
            None,
        )
        .await
        .expect("explicit request ends quiet");
    let shown = timeout(Duration::from_millis(100), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    assert!(matches!(shown, BrokerEvent::SuggestionShow { .. }));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);

    // And automatic work flows again afterwards. The context update first
    // supersedes the still-visible suggestion, so wait past its clear.
    let third = context(3, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 3), third.clone())
        .await
        .expect("third context");
    broker
        .request_suggestion(
            coordinates(session_id, 3),
            SuggestRequestPayload {
                fingerprint: third.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("automatic request after explicit");
    let shown = timeout(Duration::from_millis(100), async {
        loop {
            match events.recv().await {
                Some(BrokerEvent::SuggestionShow { .. }) => break true,
                Some(_) => {}
                None => break false,
            }
        }
    })
    .await
    .expect("show timeout");
    assert!(
        shown,
        "automatic work flows again after an explicit request"
    );
    assert_eq!(provider.calls.load(Ordering::Relaxed), 3);
}

/// Answers after `delay` and records the trigger each call received.
struct TriggerProvider {
    delay: Duration,
    triggers: std::sync::Mutex<Vec<crate::provider::RequestTrigger>>,
}

#[async_trait]
impl CompletionProvider for TriggerProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        unreachable!("the broker requests outcomes with a trigger")
    }

    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
        _allow_replacement: bool,
        trigger: crate::provider::RequestTrigger,
    ) -> Result<crate::provider::ProviderReply, ProviderError> {
        self.triggers.lock().expect("trigger log").push(trigger);
        sleep(self.delay).await;
        Ok(
            crate::provider::ProviderOutcome::Proposal(crate::provider::WritingProposal {
                text: format!(" revision {}", request.before),
                replace_before: None,
            })
            .into(),
        )
    }
}

#[test]
fn long_reading_window_is_opt_in_and_keeps_context_until_the_original_expiry() {
    let config = BrokerConfig::default().clamped();
    for adapter_kind in [
        AdapterKind::Fcitx,
        AdapterKind::Obsidian,
        AdapterKind::Terminal,
    ] {
        let mut authority = SessionAuthority {
            adapter_kind,
            capabilities: vec![Capability::Context, Capability::Suggestion],
        };
        assert_eq!(
            config.suggestion_ttl_for(&authority),
            Duration::from_secs(5)
        );
        authority.capabilities.push(Capability::LongReadingWindow);
        assert_eq!(
            config.suggestion_ttl_for(&authority),
            Duration::from_secs(15)
        );
        assert_eq!(
            config.context_authority_lease_for(&authority, config.explicit_generation_timeout),
            Duration::from_millis(16_250)
        );
    }
    // A client without a reading window keeps the configured lease.
    let other = SessionAuthority {
        adapter_kind: AdapterKind::Test,
        capabilities: vec![Capability::LongReadingWindow],
    };
    assert_eq!(
        config.suggestion_ttl_for(&other),
        Duration::from_millis(600)
    );
}

#[test]
fn explicit_generation_limit_stays_between_the_automatic_limit_and_its_ceiling() {
    let broker = |generation: u64, explicit: u64| {
        let broker = Broker::new(
            std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
            BrokerConfig {
                generation_timeout: Duration::from_millis(generation),
                explicit_generation_timeout: Duration::from_millis(explicit),
                ..BrokerConfig::default()
            },
        );
        let config = broker.inner.config;
        (
            config.generation_timeout,
            config.explicit_generation_timeout,
        )
    };
    let defaults = BrokerConfig::default();
    assert_eq!(defaults.generation_timeout, Duration::from_millis(600));
    assert_eq!(
        defaults.explicit_generation_timeout,
        Duration::from_millis(crate::engine::MAX_EXPLICIT_GENERATION_TIMEOUT_MS)
    );
    assert_eq!(
        broker(600, 5_000),
        (Duration::from_millis(600), Duration::from_millis(1_250))
    );
    assert_eq!(
        broker(400, 100),
        (Duration::from_millis(400), Duration::from_millis(400))
    );
    assert_eq!(
        broker(9_000, 9_000),
        (Duration::from_millis(600), Duration::from_millis(1_250))
    );
}

#[tokio::test]
async fn explicit_requests_get_the_longer_generation_limit() {
    for explicit in [false, true] {
        let provider = std::sync::Arc::new(TriggerProvider {
            delay: Duration::from_millis(750),
            triggers: std::sync::Mutex::new(Vec::new()),
        });
        let provider_view = std::sync::Arc::clone(&provider);
        let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
        let update = context(1, FieldPurpose::Normal);
        broker
            .update_context(coordinates(session_id, 1), update.clone())
            .await
            .expect("context");
        broker
            .request_suggestion(
                coordinates(session_id, 1),
                SuggestRequestPayload {
                    fingerprint: update.fingerprint,
                    explicit,
                },
                None,
            )
            .await
            .expect("request");
        let event = timeout(Duration::from_millis(1_500), events.recv())
            .await
            .expect("generation result")
            .expect("event");
        let breakdown = broker.metrics().snapshot().no_suggestion;
        if explicit {
            assert!(
                matches!(event, BrokerEvent::SuggestionShow { ref payload, .. }
                    if payload.text == " revision 1"),
                "{event:?}"
            );
            assert_eq!(broker.metrics().snapshot().suggestions_shown, 1);
        } else {
            assert!(
                matches!(event, BrokerEvent::SuggestionClear { ref payload, .. }
                    if payload.reason == ReasonCode::ProviderTimeout),
                "{event:?}"
            );
            assert_eq!(breakdown.expect("breakdown").timeout, 1);
        }
        assert_eq!(
            *provider_view.triggers.lock().expect("trigger log"),
            [crate::provider::RequestTrigger::from_explicit(explicit)]
        );
    }
}

#[tokio::test]
async fn control_requests_and_explicit_probes_are_explicit() {
    let provider = std::sync::Arc::new(TriggerProvider {
        delay: Duration::from_millis(750),
        triggers: std::sync::Mutex::new(Vec::new()),
    });
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Request,
                fingerprint: update.fingerprint,
                suggestion_id: None,
            },
            None,
        )
        .await
        .expect("control request");
    assert!(matches!(
        timeout(Duration::from_millis(1_500), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionShow { .. }))
    ));
    for explicit in [false, true] {
        let mut request = probe_request("Thank you", "en", false);
        request.explicit = explicit;
        let result = broker.probe(request).await.expect("probe");
        if explicit {
            assert_eq!(result.outcome, ProbeOutcome::Suggested);
            assert!(result.latency_ms.is_some_and(|latency| latency >= 750));
        } else {
            assert_eq!(result.reason, Some(NoSuggestionReason::Timeout));
        }
    }
    assert_eq!(
        *provider_view.triggers.lock().expect("trigger log"),
        [
            crate::provider::RequestTrigger::Explicit,
            crate::provider::RequestTrigger::Automatic,
            crate::provider::RequestTrigger::Explicit,
        ]
    );
}

#[test]
fn mid_sentence_space_before_a_word_survives_checked_proposal() {
    let before = "Please find the ";
    let after = "report today";
    let continuation = "final report for";
    let line = crate::writing::line_rest(after).expect("words follow the caret");
    let inserted = crate::writing::mid_line_insertion(before, line, continuation)
        .expect("the continuation rejoins the next word");
    assert_eq!(inserted, "final ");
    assert!(
        crate::writing::validate_proposal(before, after, &inserted, Some("en")).is_ok(),
        "the provider accepts the insertion before the broker's display check"
    );

    let shown = crate::engine::generation::checked_proposal(
        ProviderKind::LocalModel,
        before,
        after,
        crate::provider::WritingProposal {
            text: inserted,
            replace_before: None,
        },
    );
    let shown = shown.expect("the separating space is part of the suggestion");
    assert_eq!(shown.text, "final ");
    let parts = crate::segment::accept_word(&shown.text);
    assert_eq!(
        parts.accepted, "final ",
        "Ctrl+→ takes the last word and its space"
    );
    assert_eq!(parts.remainder, "");

    for (before, after, text) in [
        ("Please find the ", "", "final "),
        ("Thanks ", ", John", "a lot "),
        ("Please find the ", "report today", "final  "),
        ("hello", "world", "final "),
        ("hello ", " report", "final "),
        ("hello ", "report", "final\u{00a0}"),
    ] {
        assert!(
            crate::engine::generation::checked_proposal(
                ProviderKind::LocalModel,
                before,
                after,
                crate::provider::WritingProposal {
                    text: text.to_owned(),
                    replace_before: None,
                },
            )
            .is_none(),
            "rejected {before:?} | {text:?} | {after:?}"
        );
    }
}
