//! Snooze: the focused field's app or site is withheld for a while, by its
//! exact settings identity, and nothing else changes (ADR 0005).

use super::*;

fn web_target(host: &str, app_id: &str) -> TargetDescriptor {
    TargetDescriptor {
        kind: TargetKind::Browser,
        app_id: app_id.to_owned(),
        target_id: format!("field:{host}:{app_id}"),
        origin: Some(Origin {
            scheme: OriginScheme::Https,
            host: host.to_owned(),
            port: None,
        }),
    }
}

fn native_target() -> TargetDescriptor {
    TargetDescriptor {
        kind: TargetKind::DesktopApplication,
        app_id: "telegram".to_owned(),
        target_id: "ic:1".to_owned(),
        origin: None,
    }
}

/// A broker whose settings allow every Linux app and website by default.
fn open_broker() -> (tempfile::TempDir, Broker) {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    let mut settings = controlled_settings(1, true, false);
    settings.all_linux_apps = true;
    settings.all_web_origins = true;
    control_plane
        .replace_settings(0, settings)
        .expect("settings");
    let broker = Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig {
            debounce: Duration::ZERO,
            ..BrokerConfig::default()
        },
        control_plane,
    )
    .expect("controlled broker");
    (temporary, broker)
}

async fn open(
    broker: &Broker,
    target: TargetDescriptor,
) -> Result<(SessionId, mpsc::Receiver<BrokerEvent>), BrokerError> {
    let session_id = SessionId::new();
    let (sink, events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target,
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await?;
    Ok((session_id, events))
}

/// A session whose field reported focused context.
async fn focused(broker: &Broker, target: TargetDescriptor) -> SessionId {
    let (session_id, _events) = open(broker, target).await.expect("allowed session");
    broker
        .update_context(coordinates(session_id, 1), context(1, FieldPurpose::Normal))
        .await
        .expect("allowed context");
    session_id
}

fn allowed(status: &crate::protocol::PolicyStatusPayload) -> bool {
    status.context_allowed && status.display_allowed && status.suggestions_allowed
}

/// A focused field on `target` with a visible suggestion.
async fn showing(
    broker: &Broker,
    target: TargetDescriptor,
) -> (SessionId, mpsc::Receiver<BrokerEvent>) {
    let (session_id, mut events) = open(broker, target).await.expect("allowed session");
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
    let shown = timeout(Duration::from_millis(500), events.recv())
        .await
        .expect("show in time")
        .expect("show");
    assert!(matches!(shown, BrokerEvent::SuggestionShow { .. }));
    (session_id, events)
}

#[tokio::test]
async fn a_snooze_withholds_only_the_focused_site() {
    let (_temporary, broker) = open_broker();
    let mail = web_target("mail.example", "chromium");
    let (session_id, mut events) = showing(&broker, mail.clone()).await;
    let (_native, _native_events) = open(&broker, native_target()).await.expect("native");
    let epoch = broker.authority_snapshot().await.authority_epoch;

    let status = broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("one focused field");
    let origin = StableIdentity::browser_origin(
        BrowserAdapter::Chromium,
        WebScheme::Https,
        "mail.example",
        None,
    );
    assert_eq!(status.subject, origin.expect("origin"));
    assert_eq!(status.remaining_ms, 600_000);
    assert!(broker.authority_snapshot().await.authority_epoch > epoch);
    let cleared = timeout(Duration::from_millis(500), events.recv())
        .await
        .expect("clear in time")
        .expect("clear");
    let BrokerEvent::SuggestionClear { payload, .. } = cleared else {
        panic!("the visible suggestion is withdrawn")
    };
    assert_eq!(payload.reason, ReasonCode::Paused);

    let snoozed = broker.resolve_policy(&mail).await;
    assert!(
        !allowed(&snoozed) && !snoozed.paused,
        "only this site is withheld"
    );
    assert_eq!(snoozed.reason, PolicyResolutionReason::Snoozed);
    let zen = broker
        .resolve_policy(&web_target("mail.example", "zen"))
        .await;
    assert_eq!(
        zen.reason,
        PolicyResolutionReason::Snoozed,
        "one origin, any browser"
    );
    let docs = broker
        .resolve_policy(&web_target("docs.example", "chromium"))
        .await;
    assert!(allowed(&docs));
    assert!(allowed(&broker.resolve_policy(&native_target()).await));
    let next = context(2, FieldPurpose::Normal);
    assert!(
        broker
            .update_context(coordinates(session_id, 2), next)
            .await
            .is_err(),
        "the open session lost its grant"
    );
    assert!(open(&broker, mail).await.is_err(), "no new session opens");
    let health = broker.health_snapshot().await;
    assert_eq!(health.snoozes.len(), 1);
    assert!(health.snoozes[0].remaining_ms <= 600_000);
    assert!(!health.paused, "a snooze is not a pause");
}

#[tokio::test]
async fn a_snooze_ends_on_its_own_and_the_site_suggests_again() {
    let (_temporary, broker) = open_broker();
    let mail = web_target("mail.example", "chromium");
    showing(&broker, mail.clone()).await;
    let mut authority = broker.subscribe_authority_changes();
    broker
        .snooze_focused(Duration::from_millis(300))
        .await
        .expect("snooze");
    let snoozed_epoch = broker.authority_snapshot().await.authority_epoch;
    assert_eq!(
        broker.resolve_policy(&mail).await.reason,
        PolicyResolutionReason::Snoozed
    );

    // The end advances the epoch on its own, so adapters ask again.
    let ended = timeout(Duration::from_secs(2), async {
        loop {
            let event = authority.recv().await.expect("authority event");
            if event.authority_epoch > snoozed_epoch {
                return event;
            }
        }
    })
    .await
    .expect("the snooze ends on its own");
    assert!(!ended.paused);
    assert!(allowed(&broker.resolve_policy(&mail).await));
    assert_eq!(broker.health_snapshot().await.snoozes, Vec::new());
    open(&broker, mail).await.expect("the site suggests again");
}

#[tokio::test]
async fn snoozing_needs_one_focused_app_or_site_and_no_pause() {
    let (_temporary, broker) = open_broker();
    assert_eq!(
        broker.snooze_focused(Duration::from_secs(600)).await,
        Err(ReasonCode::NoContext)
    );
    open(&broker, native_target())
        .await
        .expect("unfocused session");
    assert_eq!(
        broker.snooze_focused(Duration::from_secs(600)).await,
        Err(ReasonCode::NoContext),
        "a session without focused context names nothing"
    );

    focused(&broker, web_target("mail.example", "chromium")).await;
    focused(&broker, web_target("docs.example", "chromium")).await;
    let epoch = broker.authority_snapshot().await.authority_epoch;
    assert_eq!(
        broker.snooze_focused(Duration::from_secs(600)).await,
        Err(ReasonCode::AmbiguousSession)
    );
    assert_eq!(
        broker.authority_snapshot().await.authority_epoch,
        epoch,
        "a refused snooze changes nothing"
    );

    let (_temporary, broker) = open_broker();
    focused(&broker, native_target()).await;
    focused(&broker, native_target()).await;
    let status = broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("two fields of one app");
    assert_eq!(
        status.subject,
        StableIdentity::linux_app(crate::settings::LinuxAdapter::Fcitx, "telegram").expect("app")
    );

    let (_temporary, broker) = open_broker();
    focused(&broker, native_target()).await;
    broker.set_paused(true).await.expect("pause");
    assert_eq!(
        broker.snooze_focused(Duration::from_secs(600)).await,
        Err(ReasonCode::Paused)
    );
}

#[tokio::test]
async fn ending_snoozes_restores_every_site_at_once() {
    let (_temporary, broker) = open_broker();
    let mail = web_target("mail.example", "chromium");
    focused(&broker, mail.clone()).await;
    broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("snooze");
    let snoozed_epoch = broker.authority_snapshot().await.authority_epoch;

    assert_eq!(broker.end_snoozes().await, 1);
    assert!(broker.authority_snapshot().await.authority_epoch > snoozed_epoch);
    assert!(allowed(&broker.resolve_policy(&mail).await));
    let epoch = broker.authority_snapshot().await.authority_epoch;
    assert_eq!(broker.end_snoozes().await, 0);
    assert_eq!(
        broker.authority_snapshot().await.authority_epoch,
        epoch,
        "nothing to end changes no authority"
    );
}

#[tokio::test]
async fn an_earlier_snoozes_wake_up_leaves_a_later_snooze_alone() {
    let (_temporary, broker) = open_broker();
    let telegram = native_target();
    focused(&broker, telegram.clone()).await;
    broker
        .snooze_focused(Duration::from_millis(50))
        .await
        .expect("short snooze");
    broker.end_snoozes().await;
    focused(&broker, telegram.clone()).await;
    broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("long snooze");

    sleep(Duration::from_millis(200)).await;
    assert_eq!(
        broker.resolve_policy(&telegram).await.reason,
        PolicyResolutionReason::Snoozed,
        "the short snooze's end finds the long one running"
    );
    assert_eq!(broker.health_snapshot().await.snoozes.len(), 1);
}
