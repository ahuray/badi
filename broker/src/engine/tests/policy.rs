//! Authority epochs, pause, policy resolution and settings replacement.

use super::*;

#[tokio::test]
async fn authority_ack_gate_tracks_the_current_epoch() {
    let broker = Broker::new(
        std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    // Connections without policy registration are governed by capabilities.
    broker
        .require_current_authority_ack("unregistered")
        .await
        .expect("unregistered connection is not ack-gated");

    broker.register_policy_client("conn".to_owned()).await;
    assert!(matches!(
        broker.require_current_authority_ack("conn").await,
        Err(BrokerError::Stale)
    ));
    let epoch = broker.authority_snapshot().await.authority_epoch;
    // A future epoch can never be acknowledged.
    assert!(
        broker
            .acknowledge_authority("conn", epoch + 1)
            .await
            .is_err()
    );
    broker
        .acknowledge_authority("conn", epoch)
        .await
        .expect("current epoch acknowledgement");
    broker
        .require_current_authority_ack("conn")
        .await
        .expect("acknowledged connection is admitted");

    // Advancing the epoch re-blocks until the new epoch is acknowledged;
    // re-acking the stale epoch does not admit.
    broker.set_paused(true).await.expect("pause");
    assert!(matches!(
        broker.require_current_authority_ack("conn").await,
        Err(BrokerError::Stale)
    ));
    broker
        .acknowledge_authority("conn", epoch)
        .await
        .expect("stale acknowledgement is recorded but insufficient");
    assert!(matches!(
        broker.require_current_authority_ack("conn").await,
        Err(BrokerError::Stale)
    ));
    let advanced = broker.authority_snapshot().await.authority_epoch;
    broker
        .acknowledge_authority("conn", advanced)
        .await
        .expect("advanced epoch acknowledgement");
    broker
        .require_current_authority_ack("conn")
        .await
        .expect("re-acknowledged connection is admitted");
}

#[tokio::test]
async fn hard_denied_context_reaches_no_provider_bytes() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let provider_view = std::sync::Arc::clone(&provider);
    let (broker, session_id, _events) = setup(provider, BrokerConfig::default()).await;
    let mut denied = context(1, FieldPurpose::Password);
    let mut invalid_denied = denied.clone();
    invalid_denied.before = "never-forward-this-secret".to_owned();
    assert!(
        broker
            .update_context(coordinates(session_id, 1), invalid_denied)
            .await
            .is_err()
    );
    denied.before.clear();
    denied.field.sensitive = true;
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 1), denied.clone())
            .await
            .expect("policy outcome"),
        ContextOutcome::Denied
    );
    assert!(matches!(
        broker
            .request_suggestion(
                coordinates(session_id, 1),
                SuggestRequestPayload {
                    fingerprint: denied.fingerprint,
                    explicit: true,
                },
                None,
            )
            .await,
        Err(BrokerError::NoContext)
    ));
    sleep(Duration::from_millis(150)).await;
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider_view.bytes.load(Ordering::Relaxed), 0);
    assert_eq!(broker.metrics().snapshot().provider_input_bytes, 0);
}

#[tokio::test]
async fn restrictive_session_activation_is_applied_before_context_validation() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let provider_view = std::sync::Arc::clone(&provider);
    let broker = Broker::new(provider, BrokerConfig::default());
    let session_id = SessionId::new();
    let (sink, _events) = mpsc::channel(4);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "never-field".to_owned(),
                    origin: None,
                },
                activation: Activation::Never,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(sink),
        )
        .await
        .expect("open never session");

    let mut claimed_always = context(1, FieldPurpose::Normal);
    claimed_always.before = "must-not-cross-boundary".to_owned();
    assert!(matches!(
        broker
            .update_context(coordinates(session_id, 1), claimed_always)
            .await,
        Err(BrokerError::Protocol(
            crate::protocol::ProtocolError::InvalidPayload
        ))
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 0);
    let state = broker.inner.state.lock().await;
    let session = state.sessions.get(&session_id).expect("session");
    assert_eq!(session.coordinates, coordinates(session_id, 0));
    assert!(session.context.is_none());
    assert!(!session.context_seen_at_coordinates);
}

#[tokio::test]
async fn manual_session_cannot_be_escalated_by_context_claim() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let provider_view = std::sync::Arc::clone(&provider);
    let broker = Broker::new(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
            provider_timeout: Duration::from_secs(1),
            suggestion_ttl: Duration::from_millis(100),
            ..BrokerConfig::default()
        },
    );
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::Browser,
                    app_id: "fixture-browser".to_owned(),
                    target_id: "manual-field".to_owned(),
                    origin: None,
                },
                activation: Activation::Manual,
            },
            SessionAuthority {
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(sink),
        )
        .await
        .expect("open manual session");

    let ambient = context(1, FieldPurpose::Normal);
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 1), ambient.clone())
            .await
            .expect("ambient policy"),
        ContextOutcome::ManualRequired
    );
    assert!(matches!(
        broker
            .request_suggestion(
                coordinates(session_id, 1),
                SuggestRequestPayload {
                    fingerprint: ambient.fingerprint,
                    explicit: false,
                },
                None,
            )
            .await,
        Err(BrokerError::NoContext)
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 0);

    let mut explicit = context(2, FieldPurpose::Normal);
    explicit.explicit = true;
    assert_eq!(
        broker
            .update_context(coordinates(session_id, 2), explicit.clone())
            .await
            .expect("explicit policy"),
        ContextOutcome::Allowed
    );
    broker
        .request_suggestion(
            coordinates(session_id, 2),
            SuggestRequestPayload {
                fingerprint: explicit.fingerprint,
                explicit: true,
            },
            None,
        )
        .await
        .expect("manual request");
    assert!(matches!(
        timeout(Duration::from_millis(50), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionShow { .. }))
    ));
    assert_eq!(provider_view.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn active_locator_fails_closed_when_two_sessions_claim_focus() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let (broker, first_id, _first_events) = setup(provider, BrokerConfig::default()).await;
    let first = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(first_id, 1), first)
        .await
        .expect("first context");

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
                protocol_version: 1,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![Capability::Context, Capability::Suggestion],
            },
            event_sink(second_sink),
        )
        .await
        .expect("second session");
    let second = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(second_id, 1), second)
        .await
        .expect("second context");
    assert!(broker.active_locator().await.is_none());

    let mut blurred = context(2, FieldPurpose::Normal);
    blurred.field.focused = false;
    assert_eq!(
        broker
            .update_context(coordinates(second_id, 2), blurred)
            .await
            .expect("blurred context"),
        ContextOutcome::Denied
    );
    assert_eq!(
        broker
            .active_locator()
            .await
            .expect("only first remains")
            .session_id,
        first_id
    );
}

#[tokio::test]
async fn pause_and_dismiss_cancel_eligibility() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::from_millis(5)));
    let (broker, session_id, mut events) = setup(
        provider,
        BrokerConfig {
            debounce: Duration::ZERO,
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
                fingerprint: first.fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("request");
    assert!(broker.set_paused(true).await.expect("pause"));
    sleep(Duration::from_millis(20)).await;
    assert!(events.try_recv().is_err());
    assert!(broker.active_locator().await.is_none());

    assert!(!broker.set_paused(false).await.expect("resume"));
    let second = context(2, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 2), second.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 2),
            SuggestRequestPayload {
                fingerprint: second.fingerprint.clone(),
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
            coordinates(session_id, 2),
            SessionControlRequestPayload {
                action: ControlAction::Dismiss,
                fingerprint: second.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
            },
            None,
        )
        .await
        .expect("dismiss");
    assert!(matches!(
        events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, .. })
            if payload.reason == crate::protocol::ReasonCode::Dismissed
    ));
    assert!(
        broker
            .active_locator()
            .await
            .is_some_and(|active| active.suggestion_id.is_none())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_pair_of_pause_toggles_is_linearizable() {
    let broker = Broker::new(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
    );
    let initial = broker.is_paused().await;
    let barrier = std::sync::Arc::new(Barrier::new(3));

    let first_broker = broker.clone();
    let first_barrier = barrier.clone();
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        first_broker.toggle_paused().await
    });
    let second_broker = broker.clone();
    let second_barrier = barrier.clone();
    let second = tokio::spawn(async move {
        second_barrier.wait().await;
        second_broker.toggle_paused().await
    });

    barrier.wait().await;
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first toggle task").expect("pause");
    let second = second.expect("second toggle task").expect("pause");

    assert_ne!(first, second, "each toggle must observe one transition");
    assert_eq!(broker.is_paused().await, initial);
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn durable_corrupt_store_deny_ack_advances_epoch_and_revokes_all_live_authority() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    {
        let control_plane = ControlPlane::open(paths.clone()).expect("control plane");
        control_plane
            .replace_settings(0, controlled_settings(1, true, true))
            .expect("allow controlled origin");
        control_plane
            .record_signal(
                StableIdentity::browser_origin(
                    BrowserAdapter::Chromium,
                    WebScheme::Http,
                    "localhost",
                    Some(4173),
                )
                .expect("controlled identity"),
                crate::personalization::PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("seed personalization");
    }
    let personalization_path = paths.personalization_path();
    std::fs::write(&personalization_path, b"corrupt aggregate evidence\n")
        .expect("corrupt aggregate store");
    let corrupt_bytes = std::fs::read(&personalization_path).expect("corrupt bytes");
    let control_plane =
        std::sync::Arc::new(ControlPlane::open(paths.clone()).expect("degraded control plane"));
    assert!(
        !control_plane
            .snapshot()
            .expect("degraded snapshot")
            .personalization_store_available
    );

    let provider = std::sync::Arc::new(ReadyThenPendingProvider::new());
    let broker = Broker::with_control_plane(
        provider.clone(),
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    broker
        .register_policy_client("connection-a".to_owned())
        .await;
    broker
        .register_policy_client("connection-b".to_owned())
        .await;
    let initial_authority = broker.authority_snapshot().await;
    broker
        .acknowledge_authority("connection-a", initial_authority.authority_epoch)
        .await
        .expect("first connection acknowledgement");
    broker
        .acknowledge_authority("connection-b", initial_authority.authority_epoch)
        .await
        .expect("second connection acknowledgement");
    assert_eq!(
        broker.authority_snapshot().await.pending_acknowledgements,
        0
    );

    let commit_session = SessionId::new();
    let (commit_sink, mut commit_events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(commit_session, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(commit_sink),
        )
        .await
        .expect("commit session");
    let (commit_context, suggestion_id) = prepare_commit_result_case(
        &broker,
        commit_session,
        &mut commit_events,
        ControlAction::AcceptAll,
    )
    .await;

    let provider_session = SessionId::new();
    let (provider_sink, mut provider_events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(provider_session, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(provider_sink),
        )
        .await
        .expect("provider session");
    let provider_context = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(provider_session, 1), provider_context.clone())
        .await
        .expect("provider context");
    broker
        .request_suggestion(
            coordinates(provider_session, 1),
            SuggestRequestPayload {
                fingerprint: provider_context.fingerprint.clone(),
                explicit: false,
            },
            None,
        )
        .await
        .expect("provider request");
    let provider_cancellation = wait_for_second_provider_token(&provider).await;

    let denied = controlled_settings(2, false, false);
    let acknowledged = broker
        .replace_settings(1, denied.clone())
        .await
        .expect("durable deny acknowledgement");
    assert_eq!(acknowledged.settings, denied);
    let persisted: SettingsV2 =
        serde_json::from_slice(&std::fs::read(paths.settings_path()).expect("persisted deny"))
            .expect("valid persisted deny");
    assert_eq!(persisted, denied);
    assert_eq!(
        std::fs::read(&personalization_path).expect("preserved corrupt evidence"),
        corrupt_bytes
    );

    let authority = broker.authority_snapshot().await;
    assert!(authority.authority_epoch > initial_authority.authority_epoch);
    assert_eq!(authority.settings_revision, 2);
    assert_eq!(authority.pending_acknowledgements, 2);
    assert!(!authority.control_plane_degraded);
    assert!(provider_cancellation.is_cancelled());
    assert_eq!(broker.session_count().await, 0);

    assert!(matches!(
        commit_events.recv().await,
        Some(BrokerEvent::SuggestionClear { payload, request_id, .. })
            if payload.reason == ReasonCode::PolicyNever
                && request_id.as_deref() == Some("applied-case")
    ));
    assert!(provider_events.recv().await.is_none());
    assert!(matches!(
        broker
            .commit_result(
                coordinates(commit_session, 1),
                CommitResultPayload {
                    fingerprint: commit_context.fingerprint,
                    suggestion_id,
                    status: CommitStatus::Applied,
                },
            )
            .await,
        Err(BrokerError::UnknownSession)
    ));
    assert!(matches!(
        broker
            .update_context(
                coordinates(provider_session, 2),
                context(2, FieldPurpose::Normal),
            )
            .await,
        Err(BrokerError::UnknownSession)
    ));
    let policy = broker.resolve_policy(&controlled_target()).await;
    assert!(!policy.context_allowed);
    assert!(!policy.display_allowed);
    assert!(!policy.suggestions_allowed);
    assert!(!policy.learning_allowed);
    let post_ack_session = SessionId::new();
    let (post_ack_sink, _post_ack_events) = mpsc::channel(1);
    assert!(matches!(
        broker
            .open_session(
                coordinates(post_ack_session, 0),
                SessionOpenPayload {
                    target: controlled_target(),
                    activation: Activation::Always,
                },
                controlled_authority(),
                event_sink(post_ack_sink),
            )
            .await,
        Err(BrokerError::Denied(_))
    ));
}

#[tokio::test]
async fn field_language_overrides_the_hint_without_rebinding_the_edit() {
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::new(provider.clone(), BrokerConfig::default());
    let (session_id, mut events) = open_observed_session(&broker, "field-language").await;
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "Vielen Dank für Ihre freundliche Nachricht.".to_owned();
    let fingerprint = update.fingerprint.clone();
    broker
        .update_context(coordinates(session_id, 1), update)
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: fingerprint.clone(),
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
        panic!("suggestion");
    };
    assert_eq!(payload.fingerprint, fingerprint);
    assert_eq!(
        *provider.languages.lock().expect("languages"),
        [Some("de".to_owned())]
    );
}

#[tokio::test]
async fn persisted_policy_is_enforced_and_replacement_revokes_live_sessions() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    let mut settings = controlled_settings(1, true, false);
    settings.language_overrides.insert(
        "chromium".to_owned(),
        crate::settings::PredictionLanguage::German,
    );
    control_plane
        .replace_settings(0, settings)
        .expect("allow controlled origin");
    let provider = std::sync::Arc::new(CountingProvider::new(Duration::ZERO));
    let broker = Broker::with_control_plane(
        provider.clone(),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    assert_eq!(
        broker
            .resolve_policy(&controlled_target())
            .await
            .language_override,
        Some(crate::settings::PredictionLanguage::German)
    );
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
    let mut update = context(1, FieldPurpose::Normal);
    update.before = "Please remember the importance of testing.".to_owned();
    broker
        .update_context(coordinates(session_id, 1), update)
        .await
        .expect("allowed context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: context(1, FieldPurpose::Normal).fingerprint,
                explicit: false,
            },
            None,
        )
        .await
        .expect("language override request");
    assert!(matches!(
        timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("suggestion timeout"),
        Some(BrokerEvent::SuggestionShow { .. })
    ));
    assert_eq!(
        *provider.languages.lock().expect("languages"),
        [Some("de".to_owned())]
    );

    let snapshot = broker
        .replace_settings(1, controlled_settings(2, false, false))
        .await
        .expect("replace settings");
    assert_eq!(snapshot.settings.revision, 2);
    assert_eq!(broker.session_count().await, 0);
    assert!(matches!(
        broker
            .update_context(coordinates(session_id, 2), context(2, FieldPurpose::Normal),)
            .await,
        Err(BrokerError::UnknownSession)
    ));

    let denied_session = SessionId::new();
    let (sink, _events) = mpsc::channel(8);
    assert!(matches!(
        broker
            .open_session(
                coordinates(denied_session, 0),
                SessionOpenPayload {
                    target: controlled_target(),
                    activation: Activation::Always,
                },
                controlled_authority(),
                event_sink(sink),
            )
            .await,
        Err(BrokerError::Denied(_))
    ));
}

#[tokio::test]
async fn all_linux_apps_reports_its_default_so_adapters_can_tell_it_from_a_rule() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    let mut settings = controlled_settings(1, false, false);
    settings.all_linux_apps = true;
    settings.all_web_origins = true;
    control_plane
        .replace_settings(0, settings)
        .expect("all-app default");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let app = |app_id: &str| TargetDescriptor {
        kind: TargetKind::DesktopApplication,
        app_id: app_id.to_owned(),
        target_id: "ic:1".to_owned(),
        origin: None,
    };
    let unlisted = broker.resolve_policy(&app("libreoffice")).await;
    assert!(unlisted.context_allowed && unlisted.suggestions_allowed && !unlisted.learning_allowed);
    assert_eq!(unlisted.reason, PolicyResolutionReason::MatchedDefault);
    let site = broker
        .resolve_policy(&TargetDescriptor {
            origin: Some(Origin {
                scheme: OriginScheme::Https,
                host: "mail.example".to_owned(),
                port: None,
            }),
            ..controlled_target()
        })
        .await;
    assert_eq!(
        site.reason,
        PolicyResolutionReason::MatchedRule,
        "website defaults keep the protocol v1 reason"
    );
}

#[tokio::test]
async fn all_web_origins_opens_unlisted_sites_but_keeps_exact_and_field_denial() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    // The controlled localhost origin carries an exact block.
    let mut settings = controlled_settings(1, false, false);
    settings.all_web_origins = true;
    control_plane
        .replace_settings(0, settings)
        .expect("all-web default");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let unlisted = TargetDescriptor {
        origin: Some(Origin {
            scheme: OriginScheme::Https,
            host: "mail.example".to_owned(),
            port: None,
        }),
        ..controlled_target()
    };
    let policy = broker.resolve_policy(&unlisted).await;
    assert!(policy.context_allowed && policy.suggestions_allowed);
    assert!(!policy.learning_allowed);
    let blocked = broker.resolve_policy(&controlled_target()).await;
    assert!(!blocked.context_allowed && !blocked.suggestions_allowed);
    let native = broker
        .resolve_policy(&TargetDescriptor {
            kind: TargetKind::DesktopApplication,
            app_id: "brave-browser".to_owned(),
            target_id: "ic:1".to_owned(),
            origin: None,
        })
        .await;
    assert!(!native.context_allowed, "native apps keep exact app rules");

    for (target, allowed) in [(unlisted, true), (controlled_target(), false)] {
        let session_id = SessionId::new();
        let (sink, _events) = mpsc::channel(8);
        let opened = broker
            .open_session(
                coordinates(session_id, 0),
                SessionOpenPayload {
                    target,
                    activation: Activation::Always,
                },
                controlled_authority(),
                event_sink(sink),
            )
            .await;
        assert_eq!(opened.is_ok(), allowed);
        if !allowed {
            continue;
        }
        assert_eq!(
            broker
                .update_context(coordinates(session_id, 1), context(1, FieldPurpose::Normal))
                .await
                .expect("allowed context"),
            ContextOutcome::Allowed
        );
        let mut sensitive = context(2, FieldPurpose::Password);
        sensitive.before.clear();
        sensitive.field.sensitive = true;
        assert_eq!(
            broker
                .update_context(coordinates(session_id, 2), sensitive)
                .await
                .expect("sensitive context"),
            ContextOutcome::Denied
        );
    }
}

#[tokio::test]
async fn rejected_settings_cas_is_fail_closed_while_restoring_old_authority() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("allow controlled origin");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let mut changes = broker.subscribe_authority_changes();

    let replacement = broker
        .replace_settings(99, controlled_settings(100, false, false))
        .await;
    assert!(matches!(replacement, Err(BrokerError::ControlPlane(_))));

    let gated = changes.recv().await.expect("mutation gate event");
    let restored = changes.recv().await.expect("restored authority event");
    assert!(gated.paused);
    assert!(!restored.paused);
    assert_eq!(restored.settings_revision, 1);
    let authority = broker.authority_snapshot().await;
    assert_eq!(authority.settings_revision, 1);
    assert!(!authority.paused);
    assert_eq!(
        control_plane
            .snapshot()
            .expect("disk snapshot")
            .settings
            .revision,
        1
    );
}

#[tokio::test]
async fn concurrent_settings_replacements_are_serialized_through_authority_install() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("allow controlled origin");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let mut changes = broker.subscribe_authority_changes();

    let first_broker = broker.clone();
    let first = tokio::spawn(async move {
        first_broker
            .replace_settings(1, controlled_settings(2, true, false))
            .await
    });
    assert!(changes.recv().await.expect("first mutation gate").paused);
    let second_broker = broker.clone();
    let second = tokio::spawn(async move {
        second_broker
            .replace_settings(2, controlled_settings(3, false, false))
            .await
    });

    assert_eq!(
        first
            .await
            .expect("first task")
            .expect("first replace")
            .settings
            .revision,
        2
    );
    assert_eq!(
        second
            .await
            .expect("second task")
            .expect("second replace")
            .settings
            .revision,
        3
    );
    let authority = broker.authority_snapshot().await;
    assert_eq!(authority.settings_revision, 3);
    assert_eq!(
        control_plane
            .snapshot()
            .expect("disk snapshot")
            .settings
            .revision,
        3
    );
}

#[test]
fn the_policy_reply_says_context_words_only_where_context_is_on_and_suggestions_allowed() {
    use crate::protocol::{Origin, OriginScheme, TargetDescriptor, TargetKind};
    use crate::settings::{
        BrowserAdapter, PermissionDecision, RetentionPermission, SettingsV2, StableIdentity,
        SubjectPermissions, SubjectRule, WebScheme,
    };
    let target = |host: &str| TargetDescriptor {
        kind: TargetKind::Browser,
        app_id: "chromium".to_owned(),
        target_id: "field".to_owned(),
        origin: Some(Origin {
            scheme: OriginScheme::Https,
            host: host.to_owned(),
            port: None,
        }),
    };
    let site = |host: &str| {
        StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
            .expect("identity")
    };
    let allow = SubjectPermissions {
        suggest: PermissionDecision::Allow,
        display: PermissionDecision::Allow,
        context_read: PermissionDecision::Allow,
        learn: PermissionDecision::Block,
        retention: RetentionPermission::None,
    };
    let mut settings = SettingsV2 {
        revision: 3,
        paused: false,
        subjects: vec![SubjectRule {
            identity: site("mail.example"),
            permissions: allow,
        }],
        ..SettingsV2::deny_by_default()
    };
    let words = |settings: &SettingsV2, host: &str| {
        let status =
            super::super::authority::policy_status(false, false, 1, settings, &target(host));
        status.validate().expect("valid policy reply");
        status.context_words
    };
    assert!(!words(&settings, "mail.example"), "unchosen");
    settings.context_words = Some(true);
    assert!(words(&settings, "mail.example"));
    assert!(!words(&settings, "other.example"), "no suggestions there");
    settings.context_words_off = vec![site("mail.example")];
    assert!(!words(&settings, "mail.example"), "off for this site");
}
