//! Context leases and the commit lifecycle after acceptance.

use super::*;

#[tokio::test]
async fn abrupt_silence_expires_context_and_all_derived_authority() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, _events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(2),
            suggestion_ttl: Duration::from_millis(10),
            context_authority_lease: Duration::from_millis(500),
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
    let provider_cancellation = wait_for_provider_token(&provider_view).await;
    assert!(broker.active_locator().await.is_some());

    sleep(Duration::from_millis(600)).await;

    assert!(provider_cancellation.is_cancelled());
    assert!(broker.active_locator().await.is_none());
    assert_eq!(broker.session_count().await, 1);
    let state = broker.inner.state.lock().await;
    let session = state.sessions.get(&session_id).expect("reusable session");
    assert!(session.context.is_none());
    assert!(session.cancellation.is_none());
    assert!(session.visible.is_none());
    assert!(session.pending.is_none());
    assert!(session.context_lease_cancellation.is_none());
}

#[tokio::test]
async fn newer_context_renews_silence_lease_and_session_remains_reusable() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, _events) = setup(
        provider,
        BrokerConfig {
            context_authority_lease: Duration::from_millis(500),
            ..BrokerConfig::default()
        },
    )
    .await;
    broker
        .update_context(coordinates(session_id, 1), context(1, FieldPurpose::Normal))
        .await
        .expect("first context");
    sleep(Duration::from_millis(300)).await;
    let second = context(2, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 2), second.clone())
        .await
        .expect("newer context");
    sleep(Duration::from_millis(300)).await;

    let active = broker
        .active_locator()
        .await
        .expect("renewed active context");
    assert_eq!(active.revision, 2);
    assert_eq!(active.fingerprint, second.fingerprint);

    sleep(Duration::from_millis(250)).await;
    assert!(broker.active_locator().await.is_none());
    let third = context(3, FieldPurpose::Normal);
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 3), third.clone())
            .await
            .expect("session accepts context after expiry"),
        ContextOutcome::Allowed
    );
    assert_eq!(
        broker
            .active_locator()
            .await
            .expect("reactivated session")
            .fingerprint,
        third.fingerprint
    );
}

#[tokio::test]
async fn paused_and_stale_addressed_accepts_emit_no_commit_prepare() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_secs(1),
            ..BrokerConfig::default()
        },
    )
    .await;
    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), first.clone())
        .await
        .expect("context");
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
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    let control = SessionControlRequestPayload {
        action: ControlAction::AcceptAll,
        fingerprint: first.fingerprint.clone(),
        suggestion_id: Some(payload.suggestion_id.clone()),
        word: None,
    };

    assert!(broker.set_paused(true).await);
    assert!(matches!(
        broker
            .session_control(coordinates(session_id, 1), control.clone(), None)
            .await,
        Err(BrokerError::Denied(crate::policy::PolicyReason::Paused))
    ));
    while events.try_recv().is_ok() {}
    assert!(events.try_recv().is_err());

    assert!(!broker.set_paused(false).await);
    let second = context(2, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 2), second)
        .await
        .expect("newer context");
    assert!(matches!(
        broker
            .session_control(coordinates(session_id, 1), control, None)
            .await,
        Err(BrokerError::Stale)
    ));
    assert!(events.try_recv().is_err());
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

#[tokio::test]
async fn accept_without_a_landed_commit_capability_emits_no_commit_prepare() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup_with_capabilities(
        provider,
        BrokerConfig::default(),
        vec![Capability::Context, Capability::Suggestion],
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
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };

    assert!(matches!(
        broker
            .session_control(
                coordinates(session_id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptAll,
                    fingerprint: update.fingerprint,
                    suggestion_id: Some(payload.suggestion_id),
                    word: None,
                },
                None,
            )
            .await,
        Err(BrokerError::InvalidCapability)
    ));
    assert!(events.try_recv().is_err(), "no commit grant was sent");
    let state = broker.inner.state.lock().await;
    let session = state.sessions.get(&session_id).expect("session");
    assert!(session.pending.is_none());
    drop(state);
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_prepared, 0);
    assert_eq!(metrics.commit_failures, 1);
}

#[tokio::test]
async fn expired_suggestion_is_cleared_and_ineligible() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(10),
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
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, .. }))
            if payload.reason == crate::protocol::ReasonCode::Expired
    ));
    assert!(matches!(
        broker
            .session_control(
                coordinates(session_id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptAll,
                    fingerprint: update.fingerprint,
                    suggestion_id: Some(payload.suggestion_id),
                    word: None,
                },
                None,
            )
            .await,
        Err(BrokerError::NoSuggestion)
    ));
}

#[tokio::test]
async fn ignored_commit_authorization_expires_and_becomes_ineligible() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(100),
            commit_result_lease: Duration::from_millis(10),
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
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: update.fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id.clone()),
                word: None,
            },
            Some("ignored-authorization".to_owned()),
        )
        .await
        .expect("prepare commit");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::CommitPrepare { request_id, .. })
            if request_id.as_deref() == Some("ignored-authorization")
    ));
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionClear { payload, request_id, .. }))
            if payload.reason == crate::protocol::ReasonCode::Expired
                && request_id.as_deref() == Some("ignored-authorization")
    ));
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::NoPendingCommit)
    ));
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_prepared, 1);
    assert_eq!(metrics.commit_failures, 1);
}

#[tokio::test]
async fn applied_all_retires_pre_mutation_context_without_continuation() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
            .await;

    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint,
                suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("applied all");

    assert_pre_mutation_state_retired(&broker, session_id).await;
    assert!(events.try_recv().is_err());
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_applied, 1);
    assert_eq!(metrics.word_savings.unwrap().applied, 2);
    assert_eq!(metrics.word_savings.unwrap().dispatched_unverified, 0);
    assert_eq!(metrics.commit_failures, 0);
}

#[tokio::test]
async fn an_invalid_landed_status_consumes_the_exact_commit_grant() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
            .await;
    let invalid = CommitResultPayload {
        fingerprint: update.fingerprint.clone(),
        suggestion_id: suggestion_id.clone(),
        status: CommitStatus::DispatchedUnverified,
    };
    assert!(matches!(
        broker
            .commit_result(coordinates(session_id, 1), invalid)
            .await,
        Err(BrokerError::InvalidCapability)
    ));
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::NoPendingCommit)
    ));
    assert_pre_mutation_state_retired(&broker, session_id).await;
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_prepared, 1);
    assert_eq!(metrics.commit_failures, 1);
    assert_eq!(metrics.commits_applied, 0);
}

#[tokio::test]
async fn native_dispatch_counts_words_once_without_claiming_applied_edits() {
    let broker = Broker::new(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
    );
    let (session_id, mut events) = open_observed_session(&broker, "field-1").await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
            .await;
    let result = CommitResultPayload {
        fingerprint: update.fingerprint,
        suggestion_id,
        status: CommitStatus::DispatchedUnverified,
    };
    broker
        .commit_result(coordinates(session_id, 1), result.clone())
        .await
        .expect("dispatch result");
    assert!(
        broker
            .commit_result(coordinates(session_id, 1), result)
            .await
            .is_err()
    );
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_applied, 0);
    assert_eq!(metrics.word_savings.unwrap().applied, 0);
    assert_eq!(metrics.word_savings.unwrap().dispatched_unverified, 2);
}

#[tokio::test]
async fn applied_word_without_rebind_retires_remainder_and_context() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptWord)
            .await;

    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint,
                suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("applied without rebind");

    assert_pre_mutation_state_retired(&broker, session_id).await;
    assert!(events.try_recv().is_err());
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_applied, 1);
    assert_eq!(metrics.word_savings.unwrap().applied, 1);
    assert_eq!(metrics.commit_failures, 0);
}

#[tokio::test]
async fn terminal_commit_failures_revoke_pre_commit_context_authority() {
    for status in [
        CommitStatus::Stale,
        CommitStatus::Blocked,
        CommitStatus::Failed,
    ] {
        let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
        let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
        let (update, suggestion_id) =
            prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
                .await;

        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint.clone(),
                    suggestion_id,
                    status,
                },
            )
            .await
            .expect("terminal failure result");

        assert_pre_mutation_state_retired(&broker, session_id).await;
        assert!(broker.active_locator().await.is_none());
        assert!(matches!(
            broker
                .request_suggestion(
                    coordinates(session_id, 1),
                    SuggestRequestPayload {
                        fingerprint: update.fingerprint,
                        explicit: false,
                    },
                    None,
                )
                .await,
            Err(BrokerError::NoContext)
        ));
        assert_eq!(broker.metrics().snapshot().commit_failures, 1);
        assert_eq!(
            broker.metrics().snapshot().word_savings.unwrap(),
            crate::metrics::WordSavings::default()
        );
    }
}

#[tokio::test]
async fn commit_result_rechecks_expired_lease_before_timer_runs() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
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
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: update.fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id.clone()),
                word: None,
            },
            Some("delayed-result".to_owned()),
        )
        .await
        .expect("prepare commit");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::CommitPrepare { .. })
    ));

    {
        let mut state = broker.inner.state.lock().await;
        state
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.pending.as_mut())
            .expect("pending commit")
            .expires_at = std::time::Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("monotonic clock has at least one millisecond of history");
    }
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::CommitLeaseExpired)
    ));
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, request_id, .. })
            if payload.reason == crate::protocol::ReasonCode::Expired
                && request_id.as_deref() == Some("delayed-result")
    ));
    assert_eq!(broker.metrics().snapshot().commit_failures, 1);
}

#[tokio::test]
async fn mismatched_commit_result_does_not_consume_valid_pending_lease() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(provider, BrokerConfig::default()).await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
            .await;

    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint.clone(),
                    suggestion_id: "s:different".to_owned(),
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::Stale)
    ));
    assert!(
        broker.inner.state.lock().await.sessions[&session_id]
            .pending
            .is_some()
    );

    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint,
                suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("matching result uses retained lease");
    assert_pre_mutation_state_retired(&broker, session_id).await;
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_applied, 1);
    assert_eq!(metrics.commit_failures, 1);
}

#[tokio::test]
async fn failed_commit_prepare_enqueue_rejects_without_pending_state() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
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
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    drop(events);

    assert!(matches!(
        broker
            .session_control(
                coordinates(session_id, 1),
                SessionControlRequestPayload {
                    action: ControlAction::AcceptAll,
                    fingerprint: update.fingerprint.clone(),
                    suggestion_id: Some(payload.suggestion_id.clone()),
                    word: None,
                },
                Some("closed-sink".to_owned()),
            )
            .await,
        Err(BrokerError::EventSinkClosed)
    ));
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::NoPendingCommit)
    ));
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.commits_prepared, 0);
    assert_eq!(metrics.commit_failures, 1);
}

#[tokio::test]
async fn pause_revokes_a_pending_commit_and_rejects_its_result() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(100),
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
                fingerprint: update.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    let shown = timeout(Duration::from_millis(50), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected show")
    };
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::AcceptAll,
                fingerprint: update.fingerprint.clone(),
                suggestion_id: Some(payload.suggestion_id.clone()),
                word: None,
            },
            Some("accept-before-pause".to_owned()),
        )
        .await
        .expect("accept dispatch");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::CommitPrepare { request_id, .. })
            if request_id.as_deref() == Some("accept-before-pause")
    ));

    assert!(broker.set_paused(true).await);
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, request_id, .. })
            if payload.reason == crate::protocol::ReasonCode::Paused
                && request_id.as_deref() == Some("accept-before-pause")
    ));
    assert!(matches!(
        broker
            .commit_result(
                coordinates(session_id, 1),
                CommitResultPayload {
                    fingerprint: update.fingerprint,
                    suggestion_id: payload.suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::NoPendingCommit)
    ));
}
