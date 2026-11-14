//! Empirical twin of `.cursor/skills/verify-badi-formal-withhold`.
//!
//! A settings pause, a runtime pause, a desktop hold (screen share in hold
//! mode, including the grace after its helper leaves, or an inactive
//! session) and a snooze each withhold suggestions. Ending one leaves the
//! others in force. A snooze records nothing while the broker is already
//! paused, and nothing when focused fields belong to different sites.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio::sync::mpsc;

use super::{Broker, BrokerConfig, BrokerError, BrokerEvent, CountingProvider};
use crate::control_plane::ControlPlane;
use crate::protocol::{
    Activation, DesktopSession, DesktopStatePayload, FieldPurpose, Origin, OriginScheme,
    PolicyResolutionReason, PolicyStatusPayload, ReasonCode, ScreenShare, SessionId,
    SessionOpenPayload, SuggestRequestPayload, TargetDescriptor, TargetKind,
};
use crate::settings::{ScreenShareMode, SettingsV2, StoragePaths};

const HELPER: &str = "c:withhold";
const HELPER_NEXT: &str = "c:withhold-next";

fn site(host: &str) -> TargetDescriptor {
    TargetDescriptor {
        kind: TargetKind::Browser,
        app_id: "chromium".to_owned(),
        target_id: format!("field:{host}"),
        origin: Some(Origin {
            scheme: OriginScheme::Https,
            host: host.to_owned(),
            port: None,
        }),
    }
}

fn mail() -> TargetDescriptor {
    site("mail.example")
}

fn docs() -> TargetDescriptor {
    site("docs.example")
}

fn broker_with(grace: Duration) -> (tempfile::TempDir, Broker) {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = Arc::new(ControlPlane::open(paths).expect("control plane"));
    let mut settings = super::controlled_settings(1, true, false);
    settings.all_web_origins = true;
    control_plane
        .replace_settings(0, settings)
        .expect("settings");
    let broker = Broker::with_control_plane(
        Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig {
            screen_share_grace: grace,
            ..BrokerConfig::default()
        },
        control_plane,
    )
    .expect("broker");
    (temporary, broker)
}

fn broker() -> (tempfile::TempDir, Broker) {
    broker_with(Duration::from_secs(10))
}

struct Field {
    session_id: SessionId,
    _events: mpsc::Receiver<BrokerEvent>,
}

async fn open(broker: &Broker, target: TargetDescriptor) -> Result<Field, BrokerError> {
    let session_id = SessionId::new();
    let (sink, events) = mpsc::channel(32);
    broker
        .open_session(
            super::coordinates(session_id, 0),
            SessionOpenPayload {
                target,
                activation: Activation::Always,
            },
            super::controlled_authority(),
            super::event_sink(sink),
        )
        .await?;
    Ok(Field {
        session_id,
        _events: events,
    })
}

/// Fresh context at `revision`, then a suggestion request. This is
/// `update_context` followed by `request_suggestion` (`admit_generation`).
async fn acquire(broker: &Broker, session_id: SessionId, revision: u64) -> Result<(), BrokerError> {
    let payload = super::context(revision, FieldPurpose::Normal);
    let fingerprint = payload.fingerprint.clone();
    broker
        .update_context(super::coordinates(session_id, revision), payload)
        .await?;
    broker
        .request_suggestion(
            super::coordinates(session_id, revision),
            SuggestRequestPayload {
                fingerprint,
                explicit: false,
            },
            None,
        )
        .await
}

async fn policy(broker: &Broker, target: &TargetDescriptor) -> PolicyStatusPayload {
    broker.resolve_policy(target).await
}

fn grants(status: &PolicyStatusPayload) -> bool {
    !status.paused && status.context_allowed && status.display_allowed && status.suggestions_allowed
}

fn held(status: &PolicyStatusPayload) -> bool {
    status.paused
        && !status.suggestions_allowed
        && status.reason == PolicyResolutionReason::GlobalDisabled
}

fn snoozed(status: &PolicyStatusPayload) -> bool {
    !status.paused
        && !status.suggestions_allowed
        && status.reason == PolicyResolutionReason::Snoozed
}

async fn report(broker: &Broker, connection: &str, session: DesktopSession, screen_shared: bool) {
    broker
        .report_desktop_state(
            connection,
            DesktopStatePayload {
                session,
                on_battery: false,
                power_saver: false,
                screen_shared,
            },
        )
        .await
        .expect("desktop report");
}

async fn replace(broker: &Broker, change: impl FnOnce(&mut SettingsV2)) {
    let snapshot = broker
        .control_plane_snapshot()
        .await
        .expect("settings snapshot");
    let mut next = snapshot.settings.clone();
    change(&mut next);
    next.revision = snapshot.settings.revision + 1;
    broker
        .replace_settings(snapshot.settings.revision, next)
        .await
        .expect("replace settings");
}

async fn wait_until_share_clears(broker: &Broker) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if broker.desktop_status().await.screen_share == ScreenShare::None {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the share grace did not end within its bound"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn withhold_ambiguous_focus_records_no_snooze() {
    let (_temporary, broker) = broker();
    let mail_field = open(&broker, mail()).await.expect("mail");
    let docs_field = open(&broker, docs()).await.expect("docs");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("mail focused");
    acquire(&broker, docs_field.session_id, 1)
        .await
        .expect("docs focused");
    let epoch = broker.authority_snapshot().await.authority_epoch;

    assert_eq!(
        broker.snooze_focused(Duration::from_secs(600)).await,
        Err(ReasonCode::AmbiguousSession)
    );

    assert_eq!(broker.authority_snapshot().await.authority_epoch, epoch);
    assert_eq!(broker.health_snapshot().await.snoozes, Vec::new());
    assert!(grants(&policy(&broker, &mail()).await));
    assert!(grants(&policy(&broker, &docs()).await));
}

#[tokio::test]
async fn withhold_a_pause_refuses_snooze_and_records_nothing() {
    let (_temporary, broker) = broker();
    let mail_field = open(&broker, mail()).await.expect("mail");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("focused");
    broker.set_paused(true).await;

    assert_eq!(
        broker.snooze_focused(Duration::from_secs(600)).await,
        Err(ReasonCode::Paused)
    );
    assert_eq!(broker.health_snapshot().await.snoozes, Vec::new());
    broker.set_paused(false).await;
    assert!(
        grants(&policy(&broker, &mail()).await),
        "a refused snooze must not withhold after the pause lifts"
    );

    replace(&broker, |settings| settings.paused = true).await;
    assert_eq!(
        broker.snooze_focused(Duration::from_secs(600)).await,
        Err(ReasonCode::Paused)
    );
    assert_eq!(broker.health_snapshot().await.snoozes, Vec::new());
    assert!(held(&policy(&broker, &mail()).await));
}

#[tokio::test]
async fn withhold_a_snooze_hides_only_that_site() {
    let (_temporary, broker) = broker();
    let mail_field = open(&broker, mail()).await.expect("mail");
    let docs_field = open(&broker, docs()).await.expect("docs");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("mail");
    broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("snooze mail");

    assert!(snoozed(&policy(&broker, &mail()).await));
    assert!(grants(&policy(&broker, &docs()).await));
    assert!(
        acquire(&broker, mail_field.session_id, 2).await.is_err(),
        "the snoozed site cannot admit another suggestion"
    );
    acquire(&broker, docs_field.session_id, 1)
        .await
        .expect("the other site still admits");
    assert!(!broker.health_snapshot().await.paused);
}

#[tokio::test]
async fn withhold_ending_the_share_leaves_the_snooze() {
    let (_temporary, broker) = broker();
    let mail_field = open(&broker, mail()).await.expect("mail");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("mail");
    broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("snooze");
    let docs_field = open(&broker, docs()).await.expect("docs");
    acquire(&broker, docs_field.session_id, 1)
        .await
        .expect("docs while mail is snoozed");

    report(&broker, HELPER, DesktopSession::Active, true).await;

    assert!(broker.health_snapshot().await.paused);
    assert!(held(&policy(&broker, &mail()).await));
    assert!(held(&policy(&broker, &docs()).await));
    assert!(acquire(&broker, docs_field.session_id, 2).await.is_err());

    report(&broker, HELPER, DesktopSession::Active, false).await;

    assert!(!broker.health_snapshot().await.paused);
    assert!(snoozed(&policy(&broker, &mail()).await));
    assert!(grants(&policy(&broker, &docs()).await));
    assert!(acquire(&broker, mail_field.session_id, 2).await.is_err());
    acquire(&broker, docs_field.session_id, 2)
        .await
        .expect("docs is not the snoozed site");
}

#[tokio::test]
async fn withhold_ending_the_snooze_leaves_the_share() {
    let (_temporary, broker) = broker();
    let mail_field = open(&broker, mail()).await.expect("mail");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("mail");
    broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("snooze");
    report(&broker, HELPER, DesktopSession::Active, true).await;

    assert_eq!(broker.end_snoozes().await, 1);

    assert_eq!(broker.health_snapshot().await.snoozes, Vec::new());
    assert!(broker.health_snapshot().await.paused);
    assert!(held(&policy(&broker, &mail()).await));
    assert!(held(&policy(&broker, &docs()).await));
    assert!(acquire(&broker, mail_field.session_id, 2).await.is_err());
}

#[tokio::test]
async fn withhold_ending_the_share_grace_leaves_a_settings_pause() {
    let (_temporary, broker) = broker_with(Duration::from_millis(100));
    report(&broker, HELPER, DesktopSession::Active, true).await;
    replace(&broker, |settings| settings.paused = true).await;
    broker.forget_desktop_state(HELPER).await;
    assert_eq!(
        broker.desktop_status().await.screen_share,
        ScreenShare::Held,
        "a helper that leaves mid-share keeps the grace"
    );

    wait_until_share_clears(&broker).await;

    assert_eq!(
        broker.desktop_status().await.screen_share,
        ScreenShare::None
    );
    assert!(broker.health_snapshot().await.paused);
    assert!(held(&policy(&broker, &mail()).await));
    assert!(open(&broker, mail()).await.is_err());
}

#[tokio::test]
async fn withhold_a_fresh_report_ends_share_grace_without_lifting_a_settings_pause() {
    let (_temporary, broker) = broker_with(Duration::from_secs(10));
    report(&broker, HELPER, DesktopSession::Active, true).await;
    replace(&broker, |settings| settings.paused = true).await;
    broker.forget_desktop_state(HELPER).await;
    assert_eq!(
        broker.desktop_status().await.screen_share,
        ScreenShare::Held
    );

    report(&broker, HELPER_NEXT, DesktopSession::Active, false).await;

    assert_eq!(
        broker.desktop_status().await.screen_share,
        ScreenShare::None,
        "a fresh report ends the grace immediately"
    );
    assert!(broker.health_snapshot().await.paused);
    assert!(held(&policy(&broker, &mail()).await));
}

#[tokio::test]
async fn withhold_clearing_the_runtime_pause_leaves_the_snooze() {
    let (_temporary, broker) = broker();
    let mail_field = open(&broker, mail()).await.expect("mail");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("mail");
    broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("snooze");

    broker.set_paused(true).await;
    assert!(held(&policy(&broker, &mail()).await));
    broker.set_paused(false).await;

    assert!(!broker.health_snapshot().await.paused);
    assert!(snoozed(&policy(&broker, &mail()).await));
    assert!(acquire(&broker, mail_field.session_id, 2).await.is_err());
    assert_eq!(broker.end_snoozes().await, 1);
    assert!(grants(&policy(&broker, &mail()).await));
    acquire(&broker, mail_field.session_id, 2)
        .await
        .expect("the site admits once the snooze has ended");
}

#[tokio::test]
async fn withhold_clearing_the_runtime_pause_leaves_a_settings_pause() {
    let (_temporary, broker) = broker();
    replace(&broker, |settings| settings.paused = true).await;
    broker.set_paused(true).await;
    broker.set_paused(false).await;

    assert!(broker.health_snapshot().await.paused);
    assert!(held(&policy(&broker, &mail()).await));
    assert!(open(&broker, mail()).await.is_err());

    replace(&broker, |settings| settings.paused = false).await;
    let mail_field = open(&broker, mail()).await.expect("settings pause lifted");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("admits once neither pause remains");
}

#[tokio::test]
async fn withhold_suggest_while_sharing_still_honors_a_snooze_and_an_inactive_session() {
    let (_temporary, broker) = broker();
    replace(&broker, |settings| {
        settings.screen_share = ScreenShareMode::Suggest;
    })
    .await;
    report(&broker, HELPER, DesktopSession::Active, true).await;
    assert!(!broker.health_snapshot().await.paused);
    assert_eq!(
        broker.desktop_status().await.screen_share,
        ScreenShare::Suggesting
    );
    let mail_field = open(&broker, mail()).await.expect("mail while sharing");
    acquire(&broker, mail_field.session_id, 1)
        .await
        .expect("suggest mode keeps admitting");
    let docs_field = open(&broker, docs()).await.expect("docs");

    broker
        .snooze_focused(Duration::from_secs(600))
        .await
        .expect("snooze the focused site");
    assert!(snoozed(&policy(&broker, &mail()).await));
    assert!(grants(&policy(&broker, &docs()).await));
    assert!(acquire(&broker, mail_field.session_id, 2).await.is_err());

    report(&broker, HELPER, DesktopSession::Inactive, false).await;
    assert!(broker.health_snapshot().await.paused);
    assert!(held(&policy(&broker, &mail()).await));
    assert!(held(&policy(&broker, &docs()).await));
    assert!(acquire(&broker, docs_field.session_id, 1).await.is_err());

    report(&broker, HELPER, DesktopSession::Active, true).await;
    assert!(!broker.health_snapshot().await.paused);
    assert!(snoozed(&policy(&broker, &mail()).await));
    assert!(grants(&policy(&broker, &docs()).await));
    acquire(&broker, docs_field.session_id, 1)
        .await
        .expect("docs admits again; mail stays snoozed");
}
