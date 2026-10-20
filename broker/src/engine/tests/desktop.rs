//! Desktop helper reports: session holds and battery scheduling.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

use crate::control_plane::ControlPlane;
use crate::engine::{
    Broker, BrokerConfig, BrokerError, BrokerEvent, BrokerEventSink, SessionAuthority,
};
use crate::protocol::{
    Activation, AdapterKind, AutomaticScheduling, Capability, ContextChangedPayload, Coordinates,
    DesktopSession, DesktopStatePayload, FieldDescriptor, FieldPurpose, OffsetUnit, Origin,
    OriginScheme, ProviderKind, ReasonCode, Selection, SessionId, SessionOpenPayload,
    SuggestRequestPayload, TargetDescriptor, TargetKind,
};
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
use crate::settings::{
    BatteryMode, BrowserAdapter, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA,
    SettingsV2, StableIdentity, StoragePaths, SubjectPermissions, SubjectRule, WebScheme,
};

const HELPER: &str = "c:desktop-helper";

/// Records when each call started, so a test can see when the model began.
struct TimedProvider {
    calls: AtomicU64,
    started: std::sync::Mutex<Vec<Instant>>,
}

impl TimedProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU64::new(0),
            started: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }

    fn first_start(&self) -> Instant {
        self.started.lock().expect("start times")[0]
    }
}

#[async_trait]
impl CompletionProvider for TimedProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.started
            .lock()
            .expect("start times")
            .push(Instant::now());
        Ok(Some(format!(" after {}", request.before.len())))
    }
}

fn target() -> TargetDescriptor {
    TargetDescriptor {
        kind: TargetKind::Browser,
        app_id: "chromium".to_owned(),
        target_id: "desktop-field".to_owned(),
        origin: Some(Origin {
            scheme: OriginScheme::Http,
            host: "localhost".to_owned(),
            port: Some(4173),
        }),
    }
}

fn settings(battery_mode: BatteryMode) -> SettingsV2 {
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 1,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode,
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects: vec![SubjectRule {
            identity: StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Http,
                "localhost",
                Some(4173),
            )
            .expect("identity"),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Block,
                retention: RetentionPermission::None,
            },
        }],
    }
}

struct Fixture {
    broker: Broker,
    provider: Arc<TimedProvider>,
    session_id: SessionId,
    events: mpsc::Receiver<BrokerEvent>,
    _storage: TempDir,
}

async fn fixture(battery_mode: BatteryMode) -> Fixture {
    let storage = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        storage.path().join("config/badi"),
        storage.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, settings(battery_mode))
        .expect("allow the fixture origin");
    let provider = TimedProvider::new();
    let broker = Broker::with_control_plane(
        provider.clone(),
        BrokerConfig::default(),
        Arc::clone(&control_plane),
    )
    .expect("broker");
    let session_id = SessionId::new();
    let (sink, events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: target(),
                activation: Activation::Always,
            },
            SessionAuthority {
                protocol_version: 2,
                adapter_kind: AdapterKind::Browser,
                capabilities: vec![
                    Capability::Context,
                    Capability::Suggestion,
                    Capability::CommitApplied,
                    Capability::Policy,
                ],
            },
            BrokerEventSink::new(sink, CancellationToken::new()),
        )
        .await
        .expect("open session");
    Fixture {
        broker,
        provider,
        session_id,
        events,
        _storage: storage,
    }
}

fn coordinates(session_id: SessionId, revision: u64) -> Coordinates {
    Coordinates {
        session_id,
        focus_epoch: 1,
        revision,
    }
}

fn context(revision: u64, before: &str, explicit: bool) -> ContextChangedPayload {
    ContextChangedPayload {
        fingerprint: format!("fingerprint_{revision:016}"),
        before: before.to_owned(),
        after: String::new(),
        selection: Selection {
            anchor: 0,
            head: 0,
            unit: OffsetUnit::UnicodeScalarValues,
        },
        field: FieldDescriptor {
            purpose: FieldPurpose::Normal,
            editable: true,
            multiline: true,
            composing: false,
            sensitive: false,
            identity_known: true,
            focused: true,
            lock_screen: false,
        },
        activation: if explicit {
            Activation::Manual
        } else {
            Activation::Always
        },
        explicit,
        language: Some("en".to_owned()),
        window_text: None,
    }
}

impl Fixture {
    async fn request(
        &self,
        revision: u64,
        before: &str,
        explicit: bool,
    ) -> Result<(), BrokerError> {
        let mut update = context(revision, before, explicit);
        update.selection.anchor = u64::try_from(before.chars().count()).expect("short text");
        update.selection.head = update.selection.anchor;
        let fingerprint = update.fingerprint.clone();
        self.broker
            .update_context(coordinates(self.session_id, revision), update)
            .await?;
        self.broker
            .request_suggestion(
                coordinates(self.session_id, revision),
                SuggestRequestPayload {
                    fingerprint,
                    explicit,
                },
                None,
            )
            .await
    }

    async fn next_event(&mut self) -> BrokerEvent {
        timeout(Duration::from_secs(2), self.events.recv())
            .await
            .expect("event deadline")
            .expect("event")
    }

    async fn report(&self, session: DesktopSession, on_battery: bool) {
        self.broker
            .report_desktop_state(
                HELPER,
                DesktopStatePayload {
                    session,
                    on_battery,
                    power_saver: false,
                },
            )
            .await
            .expect("desktop report");
    }
}

#[tokio::test]
async fn sleep_revokes_a_visible_suggestion_and_resume_requires_fresh_acquisition() {
    let mut fixture = fixture(BatteryMode::Normal).await;
    fixture
        .request(1, "Thank you for", false)
        .await
        .expect("request");
    let BrokerEvent::SuggestionShow { .. } = fixture.next_event().await else {
        panic!("expected a visible suggestion before sleep");
    };
    let mut authority = fixture.broker.subscribe_authority_changes();
    let before = fixture.broker.authority_snapshot().await.authority_epoch;

    fixture.report(DesktopSession::Sleeping, false).await;

    let BrokerEvent::SuggestionClear { payload, .. } = fixture.next_event().await else {
        panic!("sleep must withdraw the visible suggestion");
    };
    assert_eq!(payload.reason, ReasonCode::Paused);
    let held = authority.recv().await.expect("hold epoch");
    assert_eq!(held.authority_epoch, before + 1);
    assert!(held.paused, "adapters see a paused epoch while asleep");
    let status = fixture.broker.desktop_status().await;
    assert_eq!(status.session, DesktopSession::Sleeping);
    assert!(fixture.broker.health_snapshot().await.paused);
    // The retained context is gone: nothing may be requested on it.
    assert!(
        fixture
            .request(2, "Thank you for the", false)
            .await
            .is_err()
    );
    assert_eq!(fixture.provider.calls(), 1);

    fixture.report(DesktopSession::Active, false).await;
    let resumed = authority.recv().await.expect("resume epoch");
    assert_eq!(resumed.authority_epoch, before + 2);
    assert!(!resumed.paused);
    assert_eq!(
        fixture.broker.desktop_status().await.session,
        DesktopSession::Active
    );
}

#[tokio::test]
async fn a_closed_helper_takes_its_hold_and_power_state_with_it() {
    let fixture = fixture(BatteryMode::ExplicitOnly).await;
    let mut authority = fixture.broker.subscribe_authority_changes();
    fixture.report(DesktopSession::Inactive, true).await;
    assert!(authority.recv().await.expect("hold").paused);
    let status = fixture.broker.desktop_status().await;
    assert!(status.observed && status.low_power);
    assert_eq!(status.automatic, AutomaticScheduling::ExplicitOnly);
    assert_eq!(status.session, DesktopSession::Inactive);

    fixture.broker.forget_desktop_state(HELPER).await;
    assert!(!authority.recv().await.expect("release").paused);
    let status = fixture.broker.desktop_status().await;
    assert!(!status.observed && !status.low_power);
    assert_eq!(status.automatic, AutomaticScheduling::Normal);
    status.validate().expect("unobserved status is the default");
    // Forgetting an unknown connection changes nothing.
    let epoch = fixture.broker.authority_snapshot().await.authority_epoch;
    fixture.broker.forget_desktop_state(HELPER).await;
    assert_eq!(
        fixture.broker.authority_snapshot().await.authority_epoch,
        epoch
    );
}

#[tokio::test]
async fn power_reports_alone_never_change_the_authority_epoch() {
    let fixture = fixture(BatteryMode::Quiet).await;
    let epoch = fixture.broker.authority_snapshot().await.authority_epoch;
    fixture.report(DesktopSession::Active, true).await;
    fixture.report(DesktopSession::Active, false).await;
    assert_eq!(
        fixture.broker.authority_snapshot().await.authority_epoch,
        epoch
    );
}

#[tokio::test]
async fn quiet_battery_mode_skips_keys_typed_inside_the_typing_pause() {
    let mut fixture = fixture(BatteryMode::Quiet).await;
    fixture.report(DesktopSession::Active, true).await;
    assert_eq!(
        fixture.broker.desktop_status().await.automatic,
        AutomaticScheduling::Deferred
    );
    let requested = Instant::now();
    fixture
        .request(1, "Thank you", false)
        .await
        .expect("first key");
    fixture
        .request(2, "Thank you f", false)
        .await
        .expect("second key");
    fixture
        .request(3, "Thank you fo", false)
        .await
        .expect("third key");
    let last = Instant::now();
    loop {
        if let BrokerEvent::SuggestionShow { coordinates, .. } = fixture.next_event().await {
            assert_eq!(coordinates.revision, 3);
            break;
        }
    }
    assert_eq!(
        fixture.provider.calls(),
        1,
        "superseded keys never reach the model"
    );
    assert!(fixture.provider.first_start() >= last + crate::engine::QUIET_START_DELAY);
    assert!(fixture.provider.first_start() >= requested + crate::engine::QUIET_START_DELAY);
}

#[tokio::test]
async fn quiet_battery_mode_never_delays_an_explicit_request_or_mains_power() {
    let mut fixture = fixture(BatteryMode::Quiet).await;
    fixture.report(DesktopSession::Active, true).await;
    let requested = Instant::now();
    fixture
        .request(1, "Thank you", true)
        .await
        .expect("explicit request");
    let BrokerEvent::SuggestionShow { .. } = fixture.next_event().await else {
        panic!("expected an explicit suggestion");
    };
    assert!(fixture.provider.first_start() < requested + crate::engine::QUIET_START_DELAY);

    fixture.report(DesktopSession::Active, false).await;
    assert_eq!(
        fixture.broker.desktop_status().await.automatic,
        AutomaticScheduling::Normal
    );
    let requested = Instant::now();
    fixture
        .request(2, "Thank you for", false)
        .await
        .expect("automatic request on mains");
    loop {
        if let BrokerEvent::SuggestionShow { .. } = fixture.next_event().await {
            break;
        }
    }
    let started = fixture.provider.started.lock().expect("start times")[1];
    assert!(started < requested + crate::engine::QUIET_START_DELAY);
}

#[tokio::test]
async fn explicit_only_battery_mode_abstains_automatic_requests_before_the_model() {
    let mut fixture = fixture(BatteryMode::ExplicitOnly).await;
    fixture
        .broker
        .report_desktop_state(
            HELPER,
            DesktopStatePayload {
                session: DesktopSession::Active,
                on_battery: false,
                power_saver: true,
            },
        )
        .await
        .expect("power-saver report");
    fixture
        .request(1, "Thank you", false)
        .await
        .expect("automatic request");
    let BrokerEvent::SuggestionClear { payload, .. } = fixture.next_event().await else {
        panic!("expected an abstention");
    };
    assert_eq!(payload.reason, ReasonCode::NoSuggestion);
    assert_eq!(fixture.provider.calls(), 0);
    let breakdown = fixture
        .broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.request_abstained, 1);

    fixture
        .request(2, "Thank you", true)
        .await
        .expect("explicit request");
    let BrokerEvent::SuggestionShow { .. } = fixture.next_event().await else {
        panic!("an explicit request still reaches the model");
    };
    assert_eq!(fixture.provider.calls(), 1);
}
