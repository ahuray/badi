//! Content-free outcome aggregates, memory clearing and their fences.

use super::*;

#[tokio::test]
async fn broker_records_only_content_free_shown_and_dismissed_aggregates() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, true))
        .expect("allow aggregate recording");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
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
    let mut private_context = context(1, FieldPurpose::Normal);
    private_context.before = "private prose that must never persist".to_owned();
    private_context.selection.anchor =
        u64::try_from(private_context.before.encode_utf16().count()).expect("context length");
    private_context.selection.head = private_context.selection.anchor;
    broker
        .update_context(coordinates(session_id, 1), private_context.clone())
        .await
        .expect("allowed context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: private_context.fingerprint.clone(),
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
    let suggestion_text = payload.text.clone();
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Dismiss,
                fingerprint: private_context.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
                word: None,
            },
            None,
        )
        .await
        .expect("dismiss suggestion");

    let snapshot = broker
        .control_plane_snapshot()
        .await
        .expect("aggregate snapshot");
    assert_eq!(snapshot.personalization.records.len(), 1);
    let record = &snapshot.personalization.records[0];
    assert_eq!(record.shown, 1);
    assert_eq!(record.dismissed, 1);
    assert_eq!(record.accepted_word, 0);
    assert_eq!(record.accepted_all, 0);
    let persisted =
        std::fs::read_to_string(temporary.path().join("data/badi/personalization.json"))
            .expect("persisted aggregate");
    assert!(!persisted.contains(&private_context.before));
    assert!(!persisted.contains(&suggestion_text));
}

#[tokio::test]
async fn memory_clear_detaches_live_suggestions_from_deleted_aggregates() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, true))
        .expect("allow aggregate recording");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        control_plane,
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
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("allowed context");
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
        .expect("suggestion request");
    let shown = timeout(Duration::from_millis(100), events.recv())
        .await
        .expect("show timeout")
        .expect("show event");
    let BrokerEvent::SuggestionShow { payload, .. } = shown else {
        panic!("expected suggestion show")
    };

    let (changed, cleared) = broker
        .clear_personalization()
        .await
        .expect("clear personalization");
    assert!(changed);
    assert_eq!(cleared.personalization.records, [] as [DailyAggregate; 0]);
    broker
        .session_control(
            coordinates(session_id, 1),
            SessionControlRequestPayload {
                action: ControlAction::Dismiss,
                fingerprint: update.fingerprint,
                suggestion_id: Some(payload.suggestion_id),
                word: None,
            },
            None,
        )
        .await
        .expect("dismiss still-visible suggestion");

    let after_dismiss = broker
        .control_plane_snapshot()
        .await
        .expect("flush aggregate queue");
    assert_eq!(
        after_dismiss.personalization.records,
        [] as [DailyAggregate; 0]
    );
    assert_eq!(broker.outcome_recorder_health().write_failures, 0);
}

#[tokio::test]
async fn runtime_pause_ack_fences_pre_pause_outcome_writes() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, true))
        .expect("allow aggregate recording");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
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
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("allowed context");
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
        .expect("suggestion request");
    assert!(matches!(
        timeout(Duration::from_millis(100), events.recv()).await,
        Ok(Some(BrokerEvent::SuggestionShow { .. }))
    ));

    assert!(broker.set_paused(true).await);
    let fenced = control_plane.snapshot().expect("direct aggregate snapshot");
    assert_eq!(fenced.personalization.records[0].shown, 1);
    assert_eq!(broker.outcome_recorder_health().write_failures, 0);
}

#[tokio::test]
async fn retention_grant_scrubs_pre_consent_memory_without_post_commit_persistence() {
    let temporary = tempdir().expect("temporary directory");
    let data_dir = temporary.path().join("data/badi");
    let paths =
        StoragePaths::new(temporary.path().join("config/badi"), &data_dir).expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(
            0,
            controlled_learning_settings(1, RetentionPermission::None),
        )
        .expect("allow memory-only aggregate recording");
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
        .expect("record memory-only aggregate");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        std::sync::Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let session_id = SessionId::new();
    let (sink, _events) = mpsc::channel(8);
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

    std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o500))
        .expect("make aggregate directory read-only");
    let replacement = broker
        .replace_settings(
            1,
            controlled_learning_settings(2, RetentionPermission::Bounded { days: 30 }),
        )
        .await
        .expect("retention grant does not persist pre-consent history");
    std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore aggregate directory");
    assert_eq!(replacement.settings.revision, 2);
    assert_eq!(
        replacement.personalization.records,
        [] as [DailyAggregate; 0]
    );
    assert_eq!(broker.session_count().await, 0);
    assert!(!broker.is_paused().await);
    let policy = broker.resolve_policy(&controlled_target()).await;
    assert!(!policy.paused);
    assert!(policy.context_allowed);
}

fn broker_with_stalled_recorder(
    capacity: usize,
) -> (
    Broker,
    std::sync::mpsc::Receiver<crate::engine::outcomes::OutcomeCommand>,
) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(capacity);
    let recorder = crate::engine::outcomes::OutcomeRecorder::stalled(sender);
    let broker = Broker::build(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        None,
        None,
        Some(recorder),
    );
    (broker, receiver)
}

#[tokio::test(start_paused = true)]
async fn stalled_memory_clear_is_bounded_and_never_blocks_suggestion_state() {
    let (broker, commands) = broker_with_stalled_recorder(1);
    let clearing = tokio::spawn({
        let broker = broker.clone();
        async move { broker.clear_personalization().await }
    });
    let queued = loop {
        if let Ok(command) = commands.try_recv() {
            break command;
        }
        tokio::task::yield_now().await;
    };
    assert!(matches!(
        queued,
        crate::engine::outcomes::OutcomeCommand::Clear { .. }
    ));
    assert!(
        timeout(Duration::from_millis(10), broker.is_paused())
            .await
            .is_ok(),
        "the disk clear must run without the broker state lock"
    );
    assert!(matches!(
        clearing.await.expect("clear task"),
        Err(BrokerError::ControlPlaneTimeout)
    ));

    let (full, _commands) = broker_with_stalled_recorder(0);
    assert!(matches!(
        timeout(Duration::from_millis(10), full.clear_personalization()).await,
        Ok(Err(BrokerError::ControlPlaneUnavailable))
    ));
}

#[tokio::test(start_paused = true)]
async fn a_pause_whose_outcome_fence_times_out_still_reports_the_pause() {
    let (broker, _commands) = broker_with_stalled_recorder(4);
    // Reporting a failure here would invite a retried toggle to resume.
    assert!(broker.toggle_paused().await);
    assert!(broker.is_paused().await);
}

#[tokio::test(start_paused = true)]
async fn a_desktop_hold_whose_outcome_fence_times_out_still_reports_the_status() {
    let (broker, _commands) = broker_with_stalled_recorder(4);
    // The hold is already in force when the fence runs; reporting its
    // failure would make the session helper treat a live hold as rejected.
    let status = broker
        .report_desktop_state(
            "stalled-fence-helper",
            crate::protocol::DesktopStatePayload {
                session: crate::protocol::DesktopSession::Sleeping,
                on_battery: false,
                power_saver: false,
                screen_shared: false,
            },
        )
        .await
        .expect("desktop status after the logged fence failure");
    assert_eq!(status.session, crate::protocol::DesktopSession::Sleeping);
    assert!(broker.is_paused().await);
}

#[tokio::test]
async fn memory_clear_cannot_downgrade_an_unknown_settings_commit() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        control_plane,
    )
    .expect("controlled broker");

    broker.fail_control_plane_mutation_unknown().await;
    broker.fail_control_plane_mutation_recoverable().await;
    let (_, snapshot) = broker
        .clear_personalization()
        .await
        .expect("memory clear remains independently available");

    assert_eq!(snapshot.settings.revision, 0);
    let health = broker.health_snapshot().await;
    assert!(health.control_plane_degraded);
    assert!(health.paused);
    let state = broker.inner.state.lock().await;
    assert_eq!(
        state.control_plane_condition,
        ControlPlaneCondition::RestartRequired
    );
}
