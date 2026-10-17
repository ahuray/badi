//! Editor focus reports: an editor holding its terminal's focus withholds
//! the input method's native path, and only that path, until it lets go.

use super::*;

const NEOVIM: &str = "c:neovim-bridge";
const OTHER_NEOVIM: &str = "c:other-neovim-bridge";

fn native_target() -> TargetDescriptor {
    TargetDescriptor {
        kind: TargetKind::DesktopApplication,
        app_id: "com.mitchellh.ghostty".to_owned(),
        target_id: "ic:1".to_owned(),
        origin: None,
    }
}

fn web_target() -> TargetDescriptor {
    TargetDescriptor {
        origin: Some(Origin {
            scheme: OriginScheme::Https,
            host: "mail.example".to_owned(),
            port: None,
        }),
        ..controlled_target()
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
        BrokerConfig::default(),
        control_plane,
    )
    .expect("controlled broker");
    (temporary, broker)
}

async fn open_native(broker: &Broker, session_id: SessionId) -> Result<(), BrokerError> {
    let (sink, _events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: native_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
}

#[tokio::test]
async fn a_focused_editor_withholds_only_the_native_path() {
    let (_temporary, broker) = open_broker();
    assert!(
        broker
            .resolve_policy(&native_target())
            .await
            .context_allowed
    );
    let native_session = SessionId::new();
    open_native(&broker, native_session)
        .await
        .expect("native session before the hold");
    let epoch = broker.authority_snapshot().await.authority_epoch;

    let status = broker.report_editor_focus(NEOVIM, true).await;
    assert!(status.native_withheld);
    assert!(
        broker.authority_snapshot().await.authority_epoch > epoch,
        "every adapter resolves its policy afresh"
    );
    let native = broker.resolve_policy(&native_target()).await;
    assert!(!native.context_allowed && !native.suggestions_allowed && !native.paused);
    assert_eq!(native.reason, PolicyResolutionReason::EditorFocused);
    let web = broker.resolve_policy(&web_target()).await;
    assert!(web.context_allowed, "browsers keep their own policy");
    assert!(
        broker
            .update_context(
                coordinates(native_session, 1),
                context(1, FieldPurpose::Normal)
            )
            .await
            .is_err(),
        "a native session open before the hold loses its context"
    );
    assert!(
        open_native(&broker, SessionId::new()).await.is_err(),
        "no native session opens while an editor holds focus"
    );
}

#[tokio::test]
async fn the_hold_lasts_while_any_editor_holds_focus_and_ends_with_its_connection() {
    let (_temporary, broker) = open_broker();
    broker.report_editor_focus(NEOVIM, true).await;
    let held_epoch = broker.authority_snapshot().await.authority_epoch;
    broker.report_editor_focus(NEOVIM, true).await;
    broker.report_editor_focus(OTHER_NEOVIM, true).await;
    assert_eq!(
        broker.authority_snapshot().await.authority_epoch,
        held_epoch,
        "a repeated or second claim changes no authority"
    );

    assert!(
        broker
            .report_editor_focus(NEOVIM, false)
            .await
            .native_withheld
    );
    broker.forget_editor_focus(OTHER_NEOVIM).await;
    assert!(broker.authority_snapshot().await.authority_epoch > held_epoch);
    assert!(
        broker
            .resolve_policy(&native_target())
            .await
            .context_allowed
    );
    open_native(&broker, SessionId::new())
        .await
        .expect("native path restored");
}
