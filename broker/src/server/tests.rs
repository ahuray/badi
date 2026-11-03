use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::io::AsyncWriteExt as _;
use tokio::net::UnixStream;
use tokio::sync::Semaphore;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::{
    MAX_CONNECTIONS, MAX_SESSIONS_PER_CONNECTION, ServerError, bind_secure,
    ensure_session_capacity, serve_connection_with_timeouts,
};
use crate::control_plane::ControlPlane;
use crate::engine::{Broker, BrokerConfig};
use crate::ipc::{read_envelope, write_envelope, write_frame};
use crate::protocol::{
    Activation, AdapterDescriptor, AdapterKind, AuthorityAckPayload, AuthorityChangedPayload,
    CURRENT_PROTOCOL_VERSION, Capability, ContextChangedPayload, ControlAction,
    ControlResultPayload, Coordinates, DesktopSession, DesktopStatePayload, DesktopStatusPayload,
    EditorFocusPayload, EditorStatusPayload, EmptyPayload, ErrorPayload, FieldDescriptor,
    FieldPurpose, GlobalControlRequestPayload, HealthStatusPayload, HelloAckPayload, HelloPayload,
    MessageType, OffsetUnit, ProbeOutcome, ProbeRequestPayload, ProbeResultPayload, ProviderKind,
    ReasonCode, Selection, SessionControlRequestPayload, SessionId, SessionOpenPayload,
    SettingsReplacePayload, SettingsStatusPayload, StyleKeyPayload, SuggestRequestPayload,
    TargetDescriptor, TargetKind, WireEnvelope,
};
use crate::provider::{
    CompletionProvider, DeterministicPhraseProvider, ProviderError, ProviderRequest,
};
use crate::settings::{
    LinuxAdapter, SETTINGS_SCHEMA, SettingsV2, StableIdentity, StoragePaths, SubjectPermissions,
    SubjectRule,
};

/// The phrase fixture, answering after a delay so a test can act first.
struct DelayedPhrase(Duration);

#[async_trait::async_trait]
impl CompletionProvider for DelayedPhrase {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        sleep(self.0).await;
        DeterministicPhraseProvider::default()
            .complete(request, cancellation)
            .await
    }
}

fn broker() -> Broker {
    Broker::new(
        Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    )
}

async fn open_test_session(client: &mut UnixStream, session_id: SessionId) {
    let mut hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Test,
                name: "server-test".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![
                Capability::Context,
                Capability::Suggestion,
                Capability::Health,
                Capability::Policy,
            ],
        },
    )
    .expect("hello");
    hello.id = Some("test:hello".to_owned());
    write_envelope(client, &hello).await.expect("write hello");
    let acknowledgment = read_envelope(client)
        .await
        .expect("read hello acknowledgment")
        .expect("hello acknowledgment");
    assert_eq!(acknowledgment.message_type, MessageType::HelloAck);
    assert_eq!(acknowledgment.id, hello.id);
    let authority = read_envelope(client)
        .await
        .expect("read initial authority")
        .expect("initial authority");
    assert_eq!(authority.message_type, MessageType::AuthorityChanged);
    let authority: AuthorityChangedPayload = authority
        .decode_payload()
        .expect("initial authority payload");
    let acknowledgment = WireEnvelope::global(
        MessageType::AuthorityAck,
        1,
        &AuthorityAckPayload {
            authority_epoch: authority.authority_epoch,
        },
    )
    .expect("authority acknowledgment");
    write_envelope(client, &acknowledgment)
        .await
        .expect("write authority acknowledgment");

    reopen_test_session(client, session_id).await;
}

async fn reopen_test_session(client: &mut UnixStream, session_id: SessionId) {
    let session = WireEnvelope::session(
        MessageType::SessionOpen,
        Coordinates {
            session_id,
            focus_epoch: 1,
            revision: 0,
        },
        1,
        &SessionOpenPayload {
            target: TargetDescriptor {
                kind: TargetKind::Fixture,
                app_id: "server-test".to_owned(),
                target_id: "field-1".to_owned(),
                origin: None,
            },
            activation: Activation::Always,
        },
    )
    .expect("session open");
    write_envelope(client, &session)
        .await
        .expect("write session open");
}

async fn wait_for_sessions(broker: &Broker, expected: u64) {
    timeout(Duration::from_millis(250), async {
        loop {
            if broker.session_count().await == expected {
                break;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("session count");
}

#[tokio::test]
async fn authority_changes_retire_sessions_and_release_connection_capacity() {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let broker = broker();
    let task_broker = broker.clone();
    let shutdown = CancellationToken::new();
    let task_shutdown = shutdown.clone();
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            task_broker,
            task_shutdown,
            Duration::from_secs(1),
            Duration::from_secs(5),
        )
        .await
    });
    let session_id = SessionId::new();
    open_test_session(&mut client, session_id).await;
    wait_for_sessions(&broker, 1).await;

    for _ in 0..=MAX_SESSIONS_PER_CONNECTION {
        for paused in [true, false] {
            assert_eq!(broker.set_paused(paused).await, paused);
            let event = timeout(Duration::from_secs(1), read_envelope(&mut client))
                .await
                .expect("authority deadline")
                .expect("read authority")
                .expect("authority event");
            assert_eq!(event.message_type, MessageType::AuthorityChanged);
            assert_eq!(
                broker.session_count().await,
                0,
                "old sessions must be retired before publishing the new epoch"
            );
            let payload: AuthorityChangedPayload = event.decode_payload().expect("authority");
            let ack = WireEnvelope::global(
                MessageType::AuthorityAck,
                0,
                &AuthorityAckPayload {
                    authority_epoch: payload.authority_epoch,
                },
            )
            .expect("ack");
            write_envelope(&mut client, &ack).await.expect("write ack");
        }
        // Adapters reuse a document session after revocation. More than
        // 64 cycles also proves old bookkeeping cannot exhaust the quota.
        reopen_test_session(&mut client, session_id).await;
        wait_for_sessions(&broker, 1).await;
    }
    shutdown.cancel();
    connection
        .await
        .expect("connection task")
        .expect("clean shutdown");
    assert_eq!(broker.session_count().await, 0);
}

/// The next frame, which must arrive within a second.
async fn read_denial(client: &mut UnixStream) -> WireEnvelope {
    timeout(Duration::from_secs(1), read_envelope(client))
        .await
        .expect("denial deadline")
        .expect("read denial")
        .expect("denial event")
}

#[tokio::test]
async fn unacknowledged_authority_cannot_open_a_session() {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let broker = broker();
    let task_broker = broker.clone();
    let shutdown = CancellationToken::new();
    let task_shutdown = shutdown.clone();
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            task_broker,
            task_shutdown,
            Duration::from_secs(1),
            Duration::from_secs(5),
        )
        .await
    });
    let mut hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Test,
                name: "server-test".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![
                Capability::Context,
                Capability::Suggestion,
                Capability::Health,
                Capability::Policy,
                Capability::Control,
            ],
        },
    )
    .expect("hello");
    hello.id = Some("test:hello".to_owned());
    write_envelope(&mut client, &hello)
        .await
        .expect("write hello");
    let hello_ack = read_envelope(&mut client)
        .await
        .expect("read hello acknowledgment")
        .expect("hello acknowledgment");
    assert_eq!(hello_ack.message_type, MessageType::HelloAck);
    let authority = read_envelope(&mut client)
        .await
        .expect("read initial authority")
        .expect("initial authority");
    let authority: AuthorityChangedPayload = authority
        .decode_payload()
        .expect("initial authority payload");

    // Without authority.ack the data plane stays closed, but the
    // connection itself survives so the client can still ack.
    let session_id = SessionId::new();
    reopen_test_session(&mut client, session_id).await;
    let denied = read_denial(&mut client).await;
    assert_eq!(denied.message_type, MessageType::Error);
    assert_eq!(broker.session_count().await, 0);
    // A session action, which could request a suggestion, waits too.
    let control = WireEnvelope::session(
        MessageType::ControlRequest,
        Coordinates {
            session_id,
            focus_epoch: 1,
            revision: 0,
        },
        1,
        &SessionControlRequestPayload {
            action: ControlAction::Request,
            fingerprint: "fingerprint_server_01".to_owned(),
            suggestion_id: None,
            word: None,
        },
    )
    .expect("session control");
    write_envelope(&mut client, &control)
        .await
        .expect("write session control");
    let denied = read_denial(&mut client).await;
    let reason: ErrorPayload = denied.decode_payload().expect("denial payload");
    assert_eq!(reason.code, ReasonCode::Stale);

    let ack = WireEnvelope::global(
        MessageType::AuthorityAck,
        1,
        &AuthorityAckPayload {
            authority_epoch: authority.authority_epoch,
        },
    )
    .expect("authority acknowledgment");
    write_envelope(&mut client, &ack).await.expect("write ack");
    reopen_test_session(&mut client, session_id).await;
    wait_for_sessions(&broker, 1).await;

    shutdown.cancel();
    connection
        .await
        .expect("connection task")
        .expect("clean shutdown");
}

#[test]
fn connection_admission_is_hard_capped() {
    let admissions = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let permits = (0..MAX_CONNECTIONS)
        .map(|_| {
            Arc::clone(&admissions)
                .try_acquire_owned()
                .expect("capacity permit")
        })
        .collect::<Vec<_>>();
    assert!(Arc::clone(&admissions).try_acquire_owned().is_err());
    drop(permits);
    assert!(Arc::clone(&admissions).try_acquire_owned().is_ok());
}

#[test]
fn per_connection_session_count_is_hard_capped() {
    let sessions = (0..MAX_SESSIONS_PER_CONNECTION)
        .map(|_| SessionId::new())
        .collect::<Vec<_>>();
    assert!(ensure_session_capacity(&sessions[..sessions.len() - 1]).is_ok());
    assert!(matches!(
        ensure_session_capacity(&sessions),
        Err(ServerError::ResourceLimit)
    ));
}

#[tokio::test]
async fn hello_deadline_rejects_an_idle_new_connection() {
    let (server, _client) = UnixStream::pair().expect("Unix stream pair");
    let result = serve_connection_with_timeouts(
        server,
        broker(),
        CancellationToken::new(),
        Duration::from_millis(5),
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(result, Err(ServerError::HandshakeTimeout)));
}

#[tokio::test]
async fn every_frame_is_current_and_a_v1_frame_ends_the_connection() {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            broker(),
            CancellationToken::new(),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
    });
    let hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Fcitx,
                name: "badi-fcitx5".to_owned(),
                version: "0.1.0".to_owned(),
            },
            capabilities: vec![
                Capability::Context,
                Capability::Suggestion,
                Capability::Policy,
            ],
        },
    )
    .expect("v2 hello");
    write_envelope(&mut client, &hello)
        .await
        .expect("write v2 hello");

    let acknowledgment = read_envelope(&mut client)
        .await
        .expect("read v2 acknowledgment")
        .expect("v2 acknowledgment");
    assert_eq!(acknowledgment.v, CURRENT_PROTOCOL_VERSION);
    let payload: HelloAckPayload = acknowledgment
        .decode_payload()
        .expect("hello acknowledgment payload");
    assert_eq!(payload.selected_v, CURRENT_PROTOCOL_VERSION);
    payload.validate().expect("valid v2 acknowledgment");

    let authority = read_envelope(&mut client)
        .await
        .expect("read v2 authority")
        .expect("v2 authority");
    assert_eq!(authority.v, CURRENT_PROTOCOL_VERSION);
    assert_eq!(authority.message_type, MessageType::AuthorityChanged);

    let wrong_version = serde_json::json!({
        "v": 1, "type": "authority.ack", "mono_ms": 1, "payload": {"authority_epoch": 0},
    });
    write_frame(
        &mut client,
        &serde_json::to_vec(&wrong_version).expect("v1 frame"),
    )
    .await
    .expect("write wrong-version frame");
    let error = read_envelope(&mut client)
        .await
        .expect("read version error")
        .expect("version error");
    assert_eq!(error.v, CURRENT_PROTOCOL_VERSION);
    assert_eq!(error.message_type, MessageType::Error);

    connection
        .await
        .expect("connection task")
        .expect("clean version rejection");
}

#[tokio::test]
async fn a_v1_hello_is_refused_and_opens_nothing() {
    let hello = |frame_version: u8, version: u8| {
        serde_json::json!({
            "v": frame_version, "id": "legacy:hello", "type": "hello", "mono_ms": 0,
            "payload": {
                "min_v": version, "max_v": version,
                "adapter": {"kind": "fcitx", "name": "legacy", "version": "1"},
                "capabilities": ["context", "suggestion", "policy"],
            },
        })
    };
    // A v1 frame, and a v2 frame asking for v1.
    for (frame, frame_refusal) in [(hello(1, 1), true), (hello(2, 1), false)] {
        let broker = broker();
        let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
        let task_broker = broker.clone();
        let connection = tokio::spawn(async move {
            serve_connection_with_timeouts(
                server,
                task_broker,
                CancellationToken::new(),
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .await
        });
        write_frame(&mut client, &serde_json::to_vec(&frame).expect("frame"))
            .await
            .expect("write hello");
        let refusal = read_envelope(&mut client)
            .await
            .expect("read refusal")
            .expect("refusal");
        assert_eq!(refusal.message_type, MessageType::Error);
        assert_eq!(refusal.v, CURRENT_PROTOCOL_VERSION);
        let reason: ErrorPayload = refusal.decode_payload().expect("refusal payload");
        assert_eq!(reason.code, ReasonCode::UnsupportedVersion);
        assert!(read_envelope(&mut client).await.expect("closed").is_none());
        let ended = connection.await.expect("connection task");
        if frame_refusal {
            assert!(matches!(
                ended,
                Err(ServerError::Frame(crate::ipc::FrameError::Protocol(
                    crate::protocol::ProtocolError::UnsupportedVersion(1)
                )))
            ));
        } else {
            assert!(matches!(
                ended,
                Err(ServerError::Protocol(
                    crate::protocol::ProtocolError::VersionNegotiationFailed
                ))
            ));
        }
        assert_eq!(broker.session_count().await, 0);
        assert_eq!(
            broker.authority_snapshot().await.pending_acknowledgements,
            0
        );
    }
}

#[tokio::test]
async fn handshake_refuses_a_mixed_range_that_matches_no_versioned_schema() {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            broker(),
            CancellationToken::new(),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
    });
    let hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: 1,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Test,
                name: "mixed-range".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![Capability::Health],
        },
    )
    .expect("structurally supported hello");
    let mut hello = hello;
    hello.id = Some("test:hello".to_owned());
    write_envelope(&mut client, &hello)
        .await
        .expect("write mixed-range hello");
    // The refusal names its reason, in the current version, then closes.
    let refusal = read_envelope(&mut client)
        .await
        .expect("read refusal")
        .expect("refusal");
    assert_eq!(refusal.message_type, MessageType::Error);
    assert_eq!(refusal.v, CURRENT_PROTOCOL_VERSION);
    assert_eq!(refusal.id, hello.id);
    let reason: ErrorPayload = refusal.decode_payload().expect("refusal payload");
    assert_eq!(reason.code, ReasonCode::UnsupportedVersion);
    assert!(read_envelope(&mut client).await.expect("closed").is_none());

    assert!(matches!(
        connection.await.expect("connection task"),
        Err(ServerError::Protocol(
            crate::protocol::ProtocolError::VersionNegotiationFailed
        ))
    ));
}

#[tokio::test]
async fn the_wire_speaks_only_settings_v2() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(
            0,
            SettingsV2 {
                revision: 1,
                paused: false,
                subjects: vec![SubjectRule {
                    identity: StableIdentity::linux_app(LinuxAdapter::Fcitx, "omawrite")
                        .expect("Linux identity"),
                    permissions: SubjectPermissions::deny_all(),
                }],
                ..SettingsV2::deny_by_default()
            },
        )
        .expect("seed settings");
    let broker = Broker::with_control_plane(
        Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
        Arc::clone(&control_plane),
    )
    .expect("controlled broker");
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            broker,
            CancellationToken::new(),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
    });
    let hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Cli,
                name: "settings-cli".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![Capability::Settings],
        },
    )
    .expect("hello");
    write_envelope(&mut client, &hello)
        .await
        .expect("write hello");
    let _ = read_envelope(&mut client)
        .await
        .expect("read hello acknowledgment")
        .expect("hello acknowledgment");

    let status = exchange(&mut client, MessageType::SettingsGet, &EmptyPayload {}).await;
    let status: SettingsStatusPayload = status.decode_payload().expect("settings payload");
    assert_eq!(status.document["schema"], SETTINGS_SCHEMA);
    assert_eq!(
        status.document["subjects"]
            .as_array()
            .expect("subjects")
            .len(),
        1
    );

    // A v1 document is refused before any merge or write.
    let refused = exchange(
        &mut client,
        MessageType::SettingsReplace,
        &SettingsReplacePayload {
            expected_revision: 1,
            document: serde_json::json!({
                "schema": "badi.settings.v1",
                "revision": 2,
                "paused": false,
                "subjects": []
            }),
        },
    )
    .await;
    assert_eq!(refused.message_type, MessageType::Error);
    drop(client);
    let _ = connection.await;
    let snapshot = control_plane.snapshot().expect("final settings");
    assert_eq!(snapshot.settings.revision, 1);
    assert_eq!(snapshot.settings.subjects.len(), 1);
}

async fn connect_test_client(
    kind: AdapterKind,
    capabilities: Vec<Capability>,
) -> (UnixStream, tokio::task::JoinHandle<Result<(), ServerError>>) {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            broker(),
            CancellationToken::new(),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
    });
    let hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind,
                name: "diagnostic-test".to_owned(),
                version: "1".to_owned(),
            },
            capabilities,
        },
    )
    .expect("hello");
    write_envelope(&mut client, &hello)
        .await
        .expect("write hello");
    let acknowledgment = read_envelope(&mut client)
        .await
        .expect("read acknowledgment")
        .expect("acknowledgment");
    assert_eq!(acknowledgment.message_type, MessageType::HelloAck);
    (client, connection)
}

async fn exchange(
    client: &mut UnixStream,
    message_type: MessageType,
    payload: &impl serde::Serialize,
) -> WireEnvelope {
    let mut request = WireEnvelope::global(message_type, 1, payload).expect("request");
    request.id = Some("ctl:diagnostic".to_owned());
    write_envelope(client, &request)
        .await
        .expect("write request");
    read_envelope(client)
        .await
        .expect("read response")
        .expect("response")
}

#[tokio::test]
async fn probe_is_a_private_cli_request() {
    let probe = ProbeRequestPayload {
        before: "Thank you".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
        allow_replacement: false,
        explicit: false,
    };
    let (mut cli, connection) = connect_test_client(
        AdapterKind::Cli,
        vec![
            Capability::Control,
            Capability::Health,
            Capability::Settings,
        ],
    )
    .await;
    let response = exchange(&mut cli, MessageType::ProbeRequest, &probe).await;
    assert_eq!(response.message_type, MessageType::ProbeResult);
    assert_eq!(response.id.as_deref(), Some("ctl:diagnostic"));
    let result: ProbeResultPayload = response.decode_payload().expect("probe result");
    result.validate().expect("valid probe result");
    assert_eq!(result.outcome, ProbeOutcome::Suggested);
    assert_eq!(result.text.as_deref(), Some(" for your time"));
    let health = exchange(&mut cli, MessageType::HealthRequest, &EmptyPayload {}).await;
    let health: HealthStatusPayload = health.decode_payload().expect("health");
    assert_eq!(
        health.metrics.provider_calls, 0,
        "probes are not adapter traffic"
    );
    assert!(health.metrics.no_suggestion.is_some());
    drop(cli);
    connection.await.expect("CLI task").expect("CLI shutdown");

    for (kind, capabilities) in [
        (
            AdapterKind::Fcitx,
            vec![Capability::Health, Capability::Settings],
        ),
        (
            AdapterKind::Cli,
            vec![Capability::Control, Capability::Health],
        ),
    ] {
        let (mut client, connection) = connect_test_client(kind, capabilities).await;
        let rejected = exchange(&mut client, MessageType::ProbeRequest, &probe).await;
        assert_eq!(rejected.message_type, MessageType::Error, "{kind:?}");
        connection
            .await
            .expect("connection task")
            .expect("rejected probe closes cleanly");
    }
}

#[tokio::test]
async fn snooze_names_why_nothing_was_snoozed() {
    let snooze = GlobalControlRequestPayload {
        action: ControlAction::Snooze,
        minutes: Some(10),
    };
    let (mut cli, connection) = connect_test_client(
        AdapterKind::Cli,
        vec![Capability::Control, Capability::Health],
    )
    .await;
    let response = exchange(&mut cli, MessageType::ControlRequest, &snooze).await;
    assert_eq!(response.message_type, MessageType::ControlResult);
    let result: ControlResultPayload = response.decode_payload().expect("control result");
    assert!(!result.accepted && result.snooze.is_none());
    assert_eq!(result.reason, ReasonCode::NoContext);
    let ended = exchange(
        &mut cli,
        MessageType::ControlRequest,
        &GlobalControlRequestPayload {
            action: ControlAction::SnoozeEnd,
            minutes: None,
        },
    )
    .await;
    let ended: ControlResultPayload = ended.decode_payload().expect("end result");
    assert!(ended.accepted);
    drop(cli);
    connection.await.expect("CLI task").expect("CLI shutdown");
}

#[tokio::test]
async fn cancellation_closes_every_session_owned_by_connection() {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let broker = broker();
    let shutdown = CancellationToken::new();
    let task_broker = broker.clone();
    let task_shutdown = shutdown.clone();
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            task_broker,
            task_shutdown,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
    });
    open_test_session(&mut client, SessionId::new()).await;
    wait_for_sessions(&broker, 1).await;

    shutdown.cancel();
    connection
        .await
        .expect("connection task")
        .expect("connection shutdown");
    assert_eq!(broker.session_count().await, 0);
}

#[tokio::test]
async fn final_protocol_error_is_drained_before_connection_closes() {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let broker = broker();
    let task_broker = broker.clone();
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            task_broker,
            CancellationToken::new(),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
    });
    open_test_session(&mut client, SessionId::new()).await;
    wait_for_sessions(&broker, 1).await;

    let duplicate_hello = WireEnvelope::global(
        MessageType::Hello,
        2,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Test,
                name: "server-test".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![Capability::Context, Capability::Suggestion],
        },
    )
    .expect("duplicate hello");
    write_envelope(&mut client, &duplicate_hello)
        .await
        .expect("write duplicate hello");
    let response = timeout(Duration::from_millis(250), read_envelope(&mut client))
        .await
        .expect("final response timeout")
        .expect("read final response")
        .expect("final response");
    assert_eq!(response.message_type, MessageType::Error);

    connection
        .await
        .expect("connection task")
        .expect("protocol-error shutdown");
    assert_eq!(broker.session_count().await, 0);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn outbound_event_does_not_cancel_a_partially_read_inbound_frame() {
    let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
    let broker = Broker::new(
        Arc::new(DelayedPhrase(Duration::from_millis(40))),
        BrokerConfig::default(),
    );
    let task_broker = broker.clone();
    let connection = tokio::spawn(async move {
        serve_connection_with_timeouts(
            server,
            task_broker,
            CancellationToken::new(),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
    });
    let session_id = SessionId::new();
    open_test_session(&mut client, session_id).await;
    wait_for_sessions(&broker, 1).await;

    let coordinates = Coordinates {
        session_id,
        focus_epoch: 1,
        revision: 1,
    };
    let context = ContextChangedPayload {
        fingerprint: "frame-race-fingerprint".to_owned(),
        before: "Thank you".to_owned(),
        after: String::new(),
        selection: Selection {
            anchor: 9,
            head: 9,
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
        activation: Activation::Always,
        explicit: false,
        language: Some("en".to_owned()),
        window_text: None,
    };
    let context_envelope =
        WireEnvelope::session(MessageType::ContextChanged, coordinates, 2, &context)
            .expect("context envelope");
    write_envelope(&mut client, &context_envelope)
        .await
        .expect("write context");
    let suggestion_envelope = WireEnvelope::session(
        MessageType::SuggestRequest,
        coordinates,
        3,
        &SuggestRequestPayload {
            fingerprint: context.fingerprint,
            explicit: false,
        },
    )
    .expect("suggestion envelope");
    write_envelope(&mut client, &suggestion_envelope)
        .await
        .expect("write suggestion request");

    let mut health = WireEnvelope::global(MessageType::HealthRequest, 4, &EmptyPayload {})
        .expect("health envelope");
    health.id = Some("health-after-partial-frame".to_owned());
    let health_body = serde_json::to_vec(&health).expect("serialize health envelope");
    let health_length = u32::try_from(health_body.len())
        .expect("health frame length")
        .to_le_bytes();
    let mut health_frame = Vec::with_capacity(4 + health_body.len());
    health_frame.extend_from_slice(&health_length);
    health_frame.extend_from_slice(&health_body);

    let (mut client_reader, mut client_writer) = client.into_split();
    client_writer
        .write_all(&health_frame[..1])
        .await
        .expect("write first health-frame byte");
    let suggestion = timeout(
        Duration::from_millis(250),
        read_envelope(&mut client_reader),
    )
    .await
    .expect("suggestion event timeout")
    .expect("read suggestion event")
    .expect("suggestion event");
    assert_eq!(suggestion.message_type, MessageType::SuggestionShow);

    client_writer
        .write_all(&health_frame[1..])
        .await
        .expect("complete health frame");
    let health_response = timeout(
        Duration::from_millis(250),
        read_envelope(&mut client_reader),
    )
    .await
    .expect("health response timeout")
    .expect("read health response")
    .expect("health response");
    assert_eq!(health_response.message_type, MessageType::HealthStatus);
    assert_eq!(health_response.id, health.id);

    drop(client_writer);
    connection
        .await
        .expect("connection task")
        .expect("connection shutdown");
}

#[tokio::test]
async fn desktop_helper_hold_lasts_until_it_reports_active_or_disconnects() {
    let broker = broker();
    let (server, mut helper) = UnixStream::pair().expect("Unix stream pair");
    let mut connection = tokio::spawn(serve_connection_with_timeouts(
        server,
        broker.clone(),
        CancellationToken::new(),
        Duration::from_secs(1),
        Duration::from_millis(50),
    ));
    let hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Session,
                name: "badi-session".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![Capability::DesktopSignals],
        },
    )
    .expect("hello");
    write_envelope(&mut helper, &hello)
        .await
        .expect("write hello");
    let acknowledgment = read_envelope(&mut helper)
        .await
        .expect("read acknowledgment")
        .expect("acknowledgment");
    assert_eq!(acknowledgment.message_type, MessageType::HelloAck);
    // Reports arrive only on change, so the idle deadline spares the helper.
    assert!(
        timeout(Duration::from_millis(200), &mut connection)
            .await
            .is_err(),
        "a desktop helper outlives the idle deadline"
    );

    let reply = exchange(
        &mut helper,
        MessageType::DesktopState,
        &DesktopStatePayload {
            session: DesktopSession::Sleeping,
            on_battery: true,
            power_saver: false,
            screen_shared: false,
        },
    )
    .await;
    assert_eq!(reply.message_type, MessageType::DesktopStatus);
    assert_eq!(reply.id.as_deref(), Some("ctl:diagnostic"));
    let status: DesktopStatusPayload = reply.decode_payload().expect("desktop status");
    status.validate().expect("valid status");
    assert!(status.observed && status.low_power);
    assert_eq!(status.session, DesktopSession::Sleeping);
    assert!(broker.is_paused().await, "sleep holds every grant");

    drop(helper);
    connection
        .await
        .expect("connection task")
        .expect("helper shutdown");
    assert!(
        !broker.is_paused().await,
        "a closed helper releases its hold"
    );
    assert!(!broker.desktop_status().await.observed);
}

#[tokio::test]
async fn desktop_reports_need_the_negotiated_capability() {
    let (mut client, connection) =
        connect_test_client(AdapterKind::Cli, vec![Capability::Health]).await;
    let reply = exchange(
        &mut client,
        MessageType::DesktopState,
        &DesktopStatePayload {
            session: DesktopSession::Inactive,
            on_battery: false,
            power_saver: false,
            screen_shared: false,
        },
    )
    .await;
    assert_eq!(reply.message_type, MessageType::Error);
    drop(client);
    let _ = connection.await;

    let (mut client, connection) =
        connect_test_client(AdapterKind::Cli, vec![Capability::Health]).await;
    let health = exchange(&mut client, MessageType::HealthRequest, &EmptyPayload {}).await;
    let health: HealthStatusPayload = health.decode_payload().expect("health");
    let desktop = health.desktop.expect("health reports the desktop state");
    assert!(!desktop.observed);
    drop(client);
    let _ = connection.await;
}

#[tokio::test]
async fn only_a_settings_client_reads_or_forgets_usage_statistics() {
    // Only the settings capability reads which apps were used.
    for (kind, capabilities) in [
        (AdapterKind::Cli, vec![Capability::Control]),
        (AdapterKind::Cli, vec![Capability::Health]),
        (AdapterKind::Session, vec![Capability::DesktopSignals]),
    ] {
        for message_type in [MessageType::StatsQuery, MessageType::StatsForget] {
            let (mut client, connection) = connect_test_client(kind, capabilities.clone()).await;
            let reply = exchange(&mut client, message_type, &EmptyPayload {}).await;
            assert_eq!(reply.message_type, MessageType::Error, "{message_type:?}");
            assert_eq!(reply.payload["code"], "invalid_capability");
            drop(client);
            let _ = connection.await;
        }
    }
}

#[tokio::test]
async fn only_the_cli_reads_style_samples_and_only_style_key_delivers_a_key() {
    let key = StyleKeyPayload {
        key_id: "5a".repeat(16),
        key: "c3".repeat(32),
    };
    // The session helper may deliver a key, but never read samples. A
    // capability refusal closes the connection, so each gets its own.
    for message_type in [
        MessageType::StyleExport,
        MessageType::StyleQuery,
        MessageType::StyleForget,
    ] {
        let (mut helper, connection) = connect_test_client(
            AdapterKind::Session,
            vec![Capability::DesktopSignals, Capability::StyleKey],
        )
        .await;
        let reply = exchange(&mut helper, message_type, &EmptyPayload {}).await;
        assert_eq!(reply.message_type, MessageType::Error, "{message_type:?}");
        assert_eq!(reply.payload["code"], "invalid_capability");
        drop(helper);
        let _ = connection.await;
    }
    // A health client cannot hand over a key.
    let (mut client, connection) =
        connect_test_client(AdapterKind::Cli, vec![Capability::Health]).await;
    let reply = exchange(&mut client, MessageType::StyleKey, &key).await;
    assert_eq!(reply.message_type, MessageType::Error);
    assert_eq!(reply.payload["code"], "invalid_capability");
    drop(client);
    let _ = connection.await;
}

#[tokio::test]
async fn idle_deadline_spares_policy_subscribers_and_closes_other_clients() {
    let broker = broker();
    let spawn_connection = |stream, broker| {
        tokio::spawn(serve_connection_with_timeouts(
            stream,
            broker,
            CancellationToken::new(),
            Duration::from_secs(1),
            Duration::from_millis(50),
        ))
    };
    let (stream, mut subscriber) = UnixStream::pair().expect("Unix stream pair");
    let mut subscription = spawn_connection(stream, broker.clone());
    open_test_session(&mut subscriber, SessionId::new()).await;
    wait_for_sessions(&broker, 1).await;

    let (stream, mut client) = UnixStream::pair().expect("Unix stream pair");
    let quiet = spawn_connection(stream, broker.clone());
    let hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Test,
                name: "quiet-client".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![Capability::Health],
        },
    )
    .expect("hello");
    write_envelope(&mut client, &hello)
        .await
        .expect("write hello");
    let acknowledgment = read_envelope(&mut client)
        .await
        .expect("read hello acknowledgment")
        .expect("hello acknowledgment");
    assert_eq!(acknowledgment.message_type, MessageType::HelloAck);
    timeout(Duration::from_secs(1), quiet)
        .await
        .expect("idle client closed")
        .expect("connection task")
        .expect("idle shutdown");
    assert!(read_envelope(&mut client).await.expect("EOF").is_none());

    assert!(
        timeout(Duration::from_millis(200), &mut subscription)
            .await
            .is_err(),
        "a policy subscriber outlives the idle deadline"
    );
    assert_eq!(broker.session_count().await, 1);
    drop(subscriber);
    timeout(Duration::from_secs(1), subscription)
        .await
        .expect("subscriber closed")
        .expect("connection task")
        .expect("peer close");
    assert_eq!(broker.session_count().await, 0);
}

#[test]
fn only_per_call_accept_failures_are_retried() {
    use std::io;

    use rustix::io::Errno;

    for errno in [
        Errno::MFILE,
        Errno::NFILE,
        Errno::CONNABORTED,
        Errno::INTR,
        Errno::AGAIN,
        Errno::NOBUFS,
        Errno::NOMEM,
    ] {
        let error = io::Error::from_raw_os_error(errno.raw_os_error());
        assert!(super::is_transient_accept_error(&error), "{errno:?}");
    }
    for errno in [Errno::BADF, Errno::INVAL, Errno::NOTSOCK] {
        let error = io::Error::from_raw_os_error(errno.raw_os_error());
        assert!(!super::is_transient_accept_error(&error), "{errno:?}");
    }
    assert!(!super::is_transient_accept_error(&io::Error::other(
        "not an errno"
    )));
}

#[tokio::test]
async fn socket_and_parent_are_private_and_guard_cleans_exact_socket() {
    let temporary = tempdir().expect("temporary directory");
    let socket_path = temporary.path().join("private").join("broker.sock");
    let (listener, guard) = bind_secure(&socket_path).expect("secure listener");
    assert_eq!(
        std::fs::metadata(socket_path.parent().expect("parent"))
            .expect("parent metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&socket_path)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(listener);
    drop(guard);
    assert!(!socket_path.exists());
}

#[tokio::test]
async fn rejects_existing_public_parent_without_changing_its_mode() {
    let temporary = tempdir().expect("temporary directory");
    let parent = temporary.path().join("existing");
    std::fs::create_dir(&parent).expect("existing parent");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755))
        .expect("public permissions");
    let result = bind_secure(&parent.join("broker.sock"));
    assert!(matches!(result, Err(super::ServerError::UnsafeSocketPath)));
    assert_eq!(
        std::fs::metadata(&parent)
            .expect("parent metadata")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(!parent.join("broker.sock").exists());
}

#[tokio::test]
async fn refuses_to_unlink_an_active_socket() {
    let temporary = tempdir().expect("temporary directory");
    let socket_path = temporary.path().join("private").join("broker.sock");
    let (listener, guard) = bind_secure(&socket_path).expect("first listener");
    assert!(matches!(
        bind_secure(&socket_path),
        Err(super::ServerError::SocketAlreadyActive)
    ));
    assert!(socket_path.exists());
    drop(listener);
    drop(guard);
    assert!(!socket_path.exists());
}

#[tokio::test]
async fn replaces_only_a_stale_owned_private_socket() {
    use std::os::unix::fs::PermissionsExt as _;

    let temporary = tempdir().expect("temporary directory");
    let parent = temporary.path().join("private");
    std::fs::create_dir(&parent).expect("private parent");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
        .expect("private permissions");
    let socket_path = parent.join("broker.sock");
    let stale = std::os::unix::net::UnixListener::bind(&socket_path).expect("stale socket");
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .expect("socket permissions");
    drop(stale);

    let (listener, guard) = bind_secure(&socket_path).expect("replace stale socket");
    assert!(socket_path.exists());
    drop(listener);
    drop(guard);
    assert!(!socket_path.exists());
}

#[tokio::test]
async fn an_editor_focus_report_holds_the_native_path_until_its_connection_closes() {
    let broker = broker();
    let (server, mut editor) = UnixStream::pair().expect("Unix stream pair");
    let connection = tokio::spawn(serve_connection_with_timeouts(
        server,
        broker.clone(),
        CancellationToken::new(),
        Duration::from_secs(1),
        Duration::from_secs(1),
    ));
    let hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Neovim,
                name: "badi-neovim".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![Capability::EditorFocus],
        },
    )
    .expect("hello");
    write_envelope(&mut editor, &hello)
        .await
        .expect("write hello");
    let acknowledgment = read_envelope(&mut editor)
        .await
        .expect("read acknowledgment")
        .expect("acknowledgment");
    assert_eq!(acknowledgment.message_type, MessageType::HelloAck);

    let reply = exchange(
        &mut editor,
        MessageType::EditorFocus,
        &EditorFocusPayload { focused: true },
    )
    .await;
    assert_eq!(reply.message_type, MessageType::EditorStatus);
    let status: EditorStatusPayload = reply.decode_payload().expect("editor status");
    assert!(status.native_withheld);

    drop(editor);
    connection
        .await
        .expect("connection task")
        .expect("editor shutdown");
    let status = broker.report_editor_focus("c:probe", false).await;
    assert!(!status.native_withheld, "a closed editor releases its hold");
}

#[test]
fn only_neovim_may_negotiate_editor_focus() {
    let hello = |kind| HelloPayload {
        min_v: CURRENT_PROTOCOL_VERSION,
        max_v: CURRENT_PROTOCOL_VERSION,
        adapter: AdapterDescriptor {
            kind,
            name: "editor".to_owned(),
            version: "1".to_owned(),
        },
        capabilities: vec![Capability::EditorFocus],
    };
    assert!(hello(AdapterKind::Neovim).validate().is_ok());
    for kind in [
        AdapterKind::Fcitx,
        AdapterKind::Obsidian,
        AdapterKind::Terminal,
    ] {
        assert!(hello(kind).validate().is_err());
    }
}
