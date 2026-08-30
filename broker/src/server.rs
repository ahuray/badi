use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::engine::{Broker, BrokerError, BrokerEvent, BrokerEventSink, SessionAuthority};
use crate::ipc::{FrameError, read_envelope, verify_peer_uid, write_envelope};
use crate::policy::PolicyReason;
use crate::protocol::{
    AuthorityAckPayload, AuthorityChangedPayload, Capability, CommitResultPayload,
    ContextChangedPayload, ControlAction, ControlResultPayload, EmptyPayload, ErrorPayload,
    GlobalControlRequestPayload, HealthStatusPayload, HelloAckPayload, HelloPayload,
    MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_FRAME_BYTES, MAX_SAFE_COUNTER, MAX_SUGGESTION_CHARS,
    MAX_SUGGESTION_WORDS, MemoryStatusPayload, MessageType, PROTOCOL_VERSION, PolicyQueryPayload,
    ReasonCode, SessionClosePayload, SessionControlRequestPayload, SessionId, SessionOpenPayload,
    SettingsReplacePayload, SettingsStatusPayload, SuggestCancelPayload, SuggestRequestPayload,
    WireEnvelope,
};
use crate::settings::SettingsV1;

const MAX_CONNECTIONS: usize = 32;
const MAX_SESSIONS_PER_CONNECTION: usize = 64;
const WIRE_QUEUE_CAPACITY: usize = 64;
const EVENT_QUEUE_CAPACITY: usize = 32;
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

pub async fn run(socket_path: &Path, broker: Broker) -> Result<(), ServerError> {
    // Register both handlers before binding. Once the socket is visible, either
    // supported termination signal is therefore guaranteed to unwind through
    // this function and drop the inode-checked SocketGuard.
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let (listener, _guard) = bind_secure(socket_path)?;
    let admissions = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let shutdown = CancellationToken::new();
    let mut connections = JoinSet::new();
    let outcome = loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => break Err(ServerError::Io(error)),
                };
                let Ok(permit) = Arc::clone(&admissions).try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let connection_broker = broker.clone();
                let connection_shutdown = shutdown.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    let _ = serve_connection(stream, connection_broker, connection_shutdown).await;
                });
            }
            completed = connections.join_next(), if !connections.is_empty() => {
                let _ = completed;
            }
            _ = interrupt.recv() => {
                break Ok(());
            }
            _ = terminate.recv() => {
                break Ok(());
            }
        }
    };

    shutdown.cancel();
    broker.shutdown().await;
    while connections.join_next().await.is_some() {}
    // A connection may have been between its last cancellation check and a
    // session.open when shutdown began. The join closes owned sessions; this
    // final pass makes the server-level postcondition explicit.
    broker.shutdown().await;
    outcome
}

pub fn bind_secure(path: &Path) -> Result<(UnixListener, SocketGuard), ServerError> {
    use std::os::unix::fs::{
        DirBuilderExt as _, FileTypeExt as _, MetadataExt as _, PermissionsExt as _,
    };

    let parent = path.parent().ok_or(ServerError::InvalidSocketPath)?;
    match std::fs::symlink_metadata(parent) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700).create(parent)?;
        }
        Err(error) => return Err(error.into()),
    }
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if !parent_metadata.is_dir()
        || parent_metadata.uid() != rustix::process::getuid().as_raw()
        || parent_metadata.mode() & 0o077 != 0
    {
        return Err(ServerError::UnsafeSocketPath);
    }

    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(ServerError::UnsafeSocketPath);
        }
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => return Err(ServerError::SocketAlreadyActive),
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                let current = std::fs::symlink_metadata(path)?;
                if !current.file_type().is_socket()
                    || current.dev() != metadata.dev()
                    || current.ino() != metadata.ino()
                {
                    return Err(ServerError::UnsafeSocketPath);
                }
                std::fs::remove_file(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(ServerError::UnsafeSocketPath),
        }
    }

    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let metadata = std::fs::symlink_metadata(path)?;
    let guard = SocketGuard {
        path: path.to_path_buf(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    Ok((listener, guard))
}

#[derive(Debug)]
pub struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};

        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

async fn serve_connection(
    stream: UnixStream,
    broker: Broker,
    shutdown: CancellationToken,
) -> Result<(), ServerError> {
    serve_connection_with_timeouts(
        stream,
        broker,
        shutdown,
        HELLO_TIMEOUT,
        CONNECTION_IDLE_TIMEOUT,
    )
    .await
}

// Handshake, bounded forwarding, and teardown deliberately stay together so
// every exit is visibly covered by the same owned-session cleanup path.
#[allow(clippy::too_many_lines)]
async fn serve_connection_with_timeouts(
    mut stream: UnixStream,
    broker: Broker,
    shutdown: CancellationToken,
    hello_timeout: Duration,
    idle_timeout: Duration,
) -> Result<(), ServerError> {
    verify_peer_uid(&stream)?;
    let first = tokio::select! {
        () = shutdown.cancelled() => return Ok(()),
        incoming = time::timeout(hello_timeout, read_envelope(&mut stream)) => {
            incoming
                .map_err(|_| ServerError::HandshakeTimeout)??
                .ok_or(ServerError::HelloRequired)?
        }
    };
    if first.message_type != MessageType::Hello {
        return Err(ServerError::HelloRequired);
    }
    let hello: HelloPayload = first.decode_payload()?;
    hello.validate()?;
    let authority = SessionAuthority {
        adapter_kind: hello.adapter.kind,
        capabilities: hello.capabilities.clone(),
    };
    let connection_id = format!("c:{}", uuid::Uuid::new_v4());
    let policy_enabled = hello.capabilities.contains(&Capability::Policy);
    let mut authority_rx = broker.subscribe_authority_changes();
    let mut acknowledgment = WireEnvelope::global(
        MessageType::HelloAck,
        broker.mono_ms(),
        &HelloAckPayload {
            selected_v: PROTOCOL_VERSION,
            connection_id: connection_id.clone(),
            enabled_capabilities: hello.capabilities.clone(),
            max_frame_bytes: MAX_FRAME_BYTES,
            max_before_chars: MAX_BEFORE_CHARS,
            max_after_chars: MAX_AFTER_CHARS,
            max_suggestion_chars: MAX_SUGGESTION_CHARS,
            max_suggestion_words: MAX_SUGGESTION_WORDS,
            paused: broker.is_paused().await,
        },
    )?;
    acknowledgment.id = first.id;
    tokio::select! {
        () = shutdown.cancelled() => return Ok(()),
        outgoing = time::timeout(
            hello_timeout,
            write_envelope(&mut stream, &acknowledgment),
        ) => outgoing.map_err(|_| ServerError::HandshakeTimeout)??,
    };
    if policy_enabled {
        broker.register_policy_client(connection_id.clone()).await;
        let authority = broker.authority_snapshot().await;
        let initial = WireEnvelope::global(
            MessageType::AuthorityChanged,
            broker.mono_ms(),
            &AuthorityChangedPayload {
                authority_epoch: authority.authority_epoch,
                settings_revision: authority.settings_revision,
                paused: authority.paused,
            },
        )?;
        tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            outgoing = time::timeout(
                hello_timeout,
                write_envelope(&mut stream, &initial),
            ) => outgoing.map_err(|_| ServerError::HandshakeTimeout)??,
        };
    }

    let (mut reader, mut writer) = stream.into_split();
    // Keep frame decoding in one owned task. Cancelling read_envelope after it
    // consumed only part of a frame would discard those bytes and desynchronize
    // the stream when an outbound broker event wins the connection select.
    let (incoming_tx, mut incoming_rx) =
        mpsc::channel::<Result<Option<WireEnvelope>, FrameError>>(1);
    let reader_task = tokio::spawn(async move {
        loop {
            let incoming = read_envelope(&mut reader).await;
            let terminal = !matches!(incoming, Ok(Some(_)));
            if incoming_tx.send(incoming).await.is_err() || terminal {
                break;
            }
        }
    });
    let (wire_tx, mut wire_rx) = mpsc::channel::<WireEnvelope>(WIRE_QUEUE_CAPACITY);
    let mut writer_task = tokio::spawn(async move {
        while let Some(envelope) = wire_rx.recv().await {
            write_envelope(&mut writer, &envelope).await?;
        }
        Ok::<(), FrameError>(())
    });
    let (event_tx, mut event_rx) = mpsc::channel::<BrokerEvent>(EVENT_QUEUE_CAPACITY);
    let connection_lifetime = CancellationToken::new();
    let event_sink = BrokerEventSink::new(event_tx.clone(), connection_lifetime.clone());

    let mut owned_sessions = Vec::<SessionId>::new();
    let idle_deadline = time::sleep(idle_timeout);
    tokio::pin!(idle_deadline);
    let outcome = loop {
        tokio::select! {
            () = shutdown.cancelled() => break Ok(()),
            () = connection_lifetime.cancelled() => break Ok(()),
            () = wire_tx.closed() => break Ok(()),
            () = &mut idle_deadline => break Ok(()),
            event = event_rx.recv() => {
                let Some(event) = event else {
                    break Ok(());
                };
                let envelope = match event.into_wire(broker.mono_ms()) {
                    Ok(envelope) => envelope,
                    Err(error) => break Err(ServerError::Protocol(error)),
                };
                if wire_tx.try_send(envelope).is_err() {
                    break Ok(());
                }
            }
            authority = authority_rx.recv(), if policy_enabled => {
                let Ok(event) = authority else {
                    break Ok(());
                };
                let envelope = WireEnvelope::global(
                    MessageType::AuthorityChanged,
                    broker.mono_ms(),
                    &event,
                )?;
                if wire_tx.try_send(envelope).is_err() {
                    break Ok(());
                }
            }
            incoming = incoming_rx.recv() => {
                let envelope = match incoming {
                    None | Some(Ok(None)) => break Ok(()),
                    Some(Ok(Some(envelope))) => envelope,
                    Some(Err(error)) => {
                        let _ = send_error(&wire_tx, &broker, None, reason_for_frame(&error));
                        break Ok(());
                    }
                };
                idle_deadline
                    .as_mut()
                    .reset(time::Instant::now() + idle_timeout);
                let request_id = envelope.id.clone();
                let request_type = envelope.message_type;
                match handle_message(
                    &broker,
                    &authority,
                    &connection_id,
                    &event_sink,
                    &mut owned_sessions,
                    envelope,
                    &wire_tx,
                )
                .await
                {
                    Ok(()) => {}
                    Err(ServerError::Broker(error)) => {
                        if send_broker_error(
                            &wire_tx,
                            &broker,
                            request_id,
                            request_type,
                            &error,
                        )
                        .await
                        .is_err()
                        {
                            break Ok(());
                        }
                    }
                    Err(error) => {
                        let _ = send_error(
                            &wire_tx,
                            &broker,
                            request_id,
                            reason_for_server(&error),
                        );
                        break Ok(());
                    }
                }
            }
        }
    };

    broker.close_owned_sessions(&owned_sessions).await;
    if policy_enabled {
        broker.unregister_policy_client(&connection_id).await;
    }
    reader_task.abort();
    let _ = reader_task.await;
    drop(event_sink);
    drop(event_tx);
    drop(wire_tx);
    if time::timeout(WRITER_DRAIN_TIMEOUT, &mut writer_task)
        .await
        .is_err()
    {
        writer_task.abort();
        let _ = writer_task.await;
    }
    outcome
}

#[allow(clippy::too_many_lines)]
async fn handle_message(
    broker: &Broker,
    authority: &SessionAuthority,
    connection_id: &str,
    event_sink: &BrokerEventSink,
    owned_sessions: &mut Vec<SessionId>,
    envelope: WireEnvelope,
    wire_tx: &mpsc::Sender<WireEnvelope>,
) -> Result<(), ServerError> {
    match envelope.message_type {
        MessageType::SessionOpen => {
            require_capability(authority, Capability::Context)?;
            require_capability(authority, Capability::Suggestion)?;
            require_capability(authority, Capability::Policy)?;
            ensure_session_capacity(owned_sessions)?;
            let coordinates = envelope.coordinates()?;
            let payload: SessionOpenPayload = envelope.decode_payload()?;
            broker
                .open_session(coordinates, payload, authority.clone(), event_sink.clone())
                .await?;
            owned_sessions.push(coordinates.session_id);
        }
        MessageType::SessionClose => {
            let coordinates = envelope.coordinates()?;
            ensure_owned(owned_sessions, coordinates.session_id)?;
            let _: SessionClosePayload = envelope.decode_payload()?;
            broker.close_session(coordinates).await?;
            owned_sessions.retain(|session_id| *session_id != coordinates.session_id);
        }
        MessageType::ContextChanged => {
            require_capability(authority, Capability::Context)?;
            let coordinates = envelope.coordinates()?;
            ensure_owned(owned_sessions, coordinates.session_id)?;
            let payload: ContextChangedPayload = envelope.decode_payload()?;
            let _ = broker.update_context(coordinates, payload).await?;
        }
        MessageType::SuggestRequest => {
            require_capability(authority, Capability::Suggestion)?;
            let coordinates = envelope.coordinates()?;
            ensure_owned(owned_sessions, coordinates.session_id)?;
            let payload: SuggestRequestPayload = envelope.decode_payload()?;
            broker
                .request_suggestion(coordinates, payload, envelope.id)
                .await?;
        }
        MessageType::SuggestCancel => {
            require_capability(authority, Capability::Suggestion)?;
            let coordinates = envelope.coordinates()?;
            ensure_owned(owned_sessions, coordinates.session_id)?;
            let payload: SuggestCancelPayload = envelope.decode_payload()?;
            broker.cancel_suggestion(coordinates, payload).await?;
        }
        MessageType::ControlRequest => {
            require_capability(authority, Capability::Control)?;
            let action: ControlAction = serde_json::from_value(
                envelope
                    .payload
                    .get("action")
                    .cloned()
                    .ok_or(ServerError::InvalidMessage)?,
            )
            .map_err(|_| ServerError::InvalidMessage)?;
            let accepted = if action.is_global() {
                let payload: GlobalControlRequestPayload = envelope.decode_payload()?;
                payload.validate()?;
                match action {
                    ControlAction::Pause => broker.set_paused(true).await,
                    ControlAction::Resume => broker.set_paused(false).await,
                    ControlAction::PauseToggle => broker.toggle_paused().await,
                    ControlAction::Request
                    | ControlAction::AcceptWord
                    | ControlAction::AcceptAll
                    | ControlAction::Dismiss => return Err(ServerError::InvalidMessage),
                };
                true
            } else {
                let coordinates = envelope.coordinates()?;
                let payload: SessionControlRequestPayload = envelope.decode_payload()?;
                broker
                    .session_control(coordinates, payload, envelope.id.clone())
                    .await?;
                true
            };
            let mut result = WireEnvelope::global(
                MessageType::ControlResult,
                broker.mono_ms(),
                &ControlResultPayload {
                    action,
                    accepted,
                    reason: ReasonCode::Accepted,
                    paused: broker.is_paused().await,
                },
            )?;
            result.id = envelope.id;
            wire_tx
                .try_send(result)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        MessageType::CommitResult => {
            let coordinates = envelope.coordinates()?;
            ensure_owned(owned_sessions, coordinates.session_id)?;
            let payload: CommitResultPayload = envelope.decode_payload()?;
            broker.commit_result(coordinates, payload).await?;
        }
        MessageType::HealthRequest => {
            require_capability(authority, Capability::Health)?;
            let _: EmptyPayload = envelope.decode_payload()?;
            let health = broker.health_snapshot().await;
            let mut response = WireEnvelope::global(
                MessageType::HealthStatus,
                broker.mono_ms(),
                &HealthStatusPayload {
                    provider: health.provider,
                    paused: health.paused,
                    authority_epoch: health.authority_epoch,
                    settings_revision: health.settings_revision,
                    control_plane_degraded: health.control_plane_degraded,
                    sessions: health.sessions,
                    socket_mode: "0600".to_owned(),
                    max_frame_bytes: health.max_frame_bytes,
                    metrics: health.metrics,
                    active: health.active,
                },
            )?;
            response.id = envelope.id;
            wire_tx
                .try_send(response)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        MessageType::PolicyQuery => {
            require_capability(authority, Capability::Policy)?;
            let payload: PolicyQueryPayload = envelope.decode_payload()?;
            payload.validate()?;
            let mut response = WireEnvelope::global(
                MessageType::PolicyStatus,
                broker.mono_ms(),
                &broker.resolve_policy(&payload.target).await,
            )?;
            response.id = envelope.id;
            wire_tx
                .try_send(response)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        MessageType::AuthorityAck => {
            require_capability(authority, Capability::Policy)?;
            let payload: AuthorityAckPayload = envelope.decode_payload()?;
            payload.validate()?;
            broker
                .acknowledge_authority(connection_id, payload.authority_epoch)
                .await?;
        }
        MessageType::SettingsGet => {
            require_settings_authority(authority)?;
            let _: EmptyPayload = envelope.decode_payload()?;
            let snapshot = broker.control_plane_snapshot().await?;
            let mut response = WireEnvelope::global(
                MessageType::SettingsStatus,
                broker.mono_ms(),
                &settings_status_payload(snapshot, broker.outcome_recorder_health())?,
            )?;
            response.id = envelope.id;
            wire_tx
                .try_send(response)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        MessageType::SettingsReplace => {
            require_settings_authority(authority)?;
            let payload: SettingsReplacePayload = envelope.decode_payload()?;
            payload.validate()?;
            let next: SettingsV1 = serde_json::from_value(payload.document)
                .map_err(|_| ServerError::InvalidMessage)?;
            next.validate().map_err(|_| ServerError::InvalidMessage)?;
            let snapshot = broker
                .replace_settings(payload.expected_revision, next)
                .await?;
            let mut response = WireEnvelope::global(
                MessageType::SettingsStatus,
                broker.mono_ms(),
                &settings_status_payload(snapshot, broker.outcome_recorder_health())?,
            )?;
            response.id = envelope.id;
            wire_tx
                .try_send(response)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        MessageType::MemoryClear => {
            require_settings_authority(authority)?;
            let _: EmptyPayload = envelope.decode_payload()?;
            let (changed, snapshot) = broker.clear_personalization().await?;
            let payload = MemoryStatusPayload {
                revision: snapshot.personalization.revision,
                records: u64::try_from(snapshot.personalization.records.len())
                    .unwrap_or(MAX_SAFE_COUNTER)
                    .min(MAX_SAFE_COUNTER),
                bytes: u64::try_from(snapshot.persisted_personalization_bytes)
                    .unwrap_or(MAX_SAFE_COUNTER)
                    .min(MAX_SAFE_COUNTER),
                changed,
            };
            let mut response =
                WireEnvelope::global(MessageType::MemoryStatus, broker.mono_ms(), &payload)?;
            response.id = envelope.id;
            wire_tx
                .try_send(response)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        MessageType::Hello
        | MessageType::HelloAck
        | MessageType::SuggestionShow
        | MessageType::SuggestionClear
        | MessageType::ControlResult
        | MessageType::CommitPrepare
        | MessageType::HealthStatus
        | MessageType::PolicyStatus
        | MessageType::AuthorityChanged
        | MessageType::SettingsStatus
        | MessageType::MemoryStatus
        | MessageType::Error => return Err(ServerError::InvalidMessage),
    }
    Ok(())
}

fn require_settings_authority(authority: &SessionAuthority) -> Result<(), ServerError> {
    require_capability(authority, Capability::Settings)?;
    if authority.adapter_kind == crate::protocol::AdapterKind::Cli {
        Ok(())
    } else {
        Err(ServerError::InvalidCapability)
    }
}

fn settings_status_payload(
    snapshot: crate::control_plane::ControlPlaneSnapshot,
    recorder: crate::engine::OutcomeRecorderHealth,
) -> Result<SettingsStatusPayload, ServerError> {
    let payload = SettingsStatusPayload {
        document: serde_json::to_value(snapshot.settings)
            .map_err(crate::protocol::ProtocolError::from)?,
        personalization_revision: snapshot.personalization.revision,
        personalization_records: u64::try_from(snapshot.personalization.records.len())
            .unwrap_or(MAX_SAFE_COUNTER)
            .min(MAX_SAFE_COUNTER),
        personalization_bytes: u64::try_from(snapshot.persisted_personalization_bytes)
            .unwrap_or(MAX_SAFE_COUNTER)
            .min(MAX_SAFE_COUNTER),
        personalization_store_available: snapshot.personalization_store_available,
        personalization_recorder_available: recorder.available,
        personalization_write_failures: recorder.write_failures.min(MAX_SAFE_COUNTER),
        personalization_dropped_signals: recorder.dropped_signals.min(MAX_SAFE_COUNTER),
    };
    payload.validate()?;
    Ok(payload)
}

fn require_capability(
    authority: &SessionAuthority,
    capability: Capability,
) -> Result<(), ServerError> {
    if authority.capabilities.contains(&capability) {
        Ok(())
    } else {
        Err(ServerError::InvalidCapability)
    }
}

fn ensure_owned(owned: &[SessionId], session_id: SessionId) -> Result<(), ServerError> {
    if owned.contains(&session_id) {
        Ok(())
    } else {
        Err(ServerError::SessionNotOwned)
    }
}

fn ensure_session_capacity(owned: &[SessionId]) -> Result<(), ServerError> {
    if owned.len() < MAX_SESSIONS_PER_CONNECTION {
        Ok(())
    } else {
        Err(ServerError::ResourceLimit)
    }
}

fn send_error(
    wire_tx: &mpsc::Sender<WireEnvelope>,
    broker: &Broker,
    request_id: Option<String>,
    reason: ReasonCode,
) -> Result<(), ServerError> {
    let mut envelope = WireEnvelope::global(
        MessageType::Error,
        broker.mono_ms(),
        &ErrorPayload::simple(reason),
    )?;
    envelope.id = request_id;
    wire_tx
        .try_send(envelope)
        .map_err(|_| ServerError::ConnectionClosed)
}

async fn send_broker_error(
    wire_tx: &mpsc::Sender<WireEnvelope>,
    broker: &Broker,
    request_id: Option<String>,
    request_type: MessageType,
    error: &BrokerError,
) -> Result<(), ServerError> {
    let mut payload = ErrorPayload::simple(reason_for_broker(error));
    if request_type == MessageType::SettingsReplace {
        let authority = broker.authority_snapshot().await;
        payload.settings_revision = Some(authority.settings_revision);
        payload.control_plane_degraded = Some(authority.control_plane_degraded);
        match error {
            BrokerError::SettingsCommittedDegraded(_) => {
                payload.code = ReasonCode::SettingsCommittedDegraded;
                payload.committed = Some(true);
            }
            BrokerError::SettingsCommitUnknown(_) | BrokerError::ControlPlaneTask => {
                payload.code = ReasonCode::SettingsCommitUnknown;
                payload.committed = None;
            }
            BrokerError::ControlPlane(crate::control_plane::ControlPlaneError::Settings(
                crate::settings::SettingsStoreError::RevisionConflict { .. },
            )) => {
                payload.code = ReasonCode::SettingsConflict;
                payload.committed = Some(false);
            }
            BrokerError::ControlPlane(_) | BrokerError::ControlPlaneUnavailable => {
                payload.code = ReasonCode::SettingsRejected;
                payload.committed = Some(false);
            }
            _ => {}
        }
    }
    payload.validate()?;
    let mut envelope = WireEnvelope::global(MessageType::Error, broker.mono_ms(), &payload)?;
    envelope.id = request_id;
    wire_tx
        .try_send(envelope)
        .map_err(|_| ServerError::ConnectionClosed)
}

const fn reason_for_broker(error: &BrokerError) -> ReasonCode {
    match error {
        BrokerError::Denied(PolicyReason::Paused) => ReasonCode::Paused,
        BrokerError::Denied(PolicyReason::FieldSensitive) => ReasonCode::FieldSensitive,
        BrokerError::Denied(PolicyReason::FieldNotEditable) => ReasonCode::FieldNotEditable,
        BrokerError::Denied(PolicyReason::FieldAmbiguous) => ReasonCode::FieldAmbiguous,
        BrokerError::Denied(PolicyReason::PolicyNever) => ReasonCode::PolicyNever,
        BrokerError::InvalidCapability => ReasonCode::InvalidCapability,
        BrokerError::CommitLeaseExpired => ReasonCode::Expired,
        BrokerError::EventSinkClosed | BrokerError::ShuttingDown => ReasonCode::SessionClosed,
        BrokerError::Denied(
            PolicyReason::AllowedAlways
            | PolicyReason::AllowedExplicit
            | PolicyReason::ManualRequired,
        )
        | BrokerError::InvalidPayload
        | BrokerError::Protocol(_)
        | BrokerError::ControlPlane(_)
        | BrokerError::ControlPlaneTask
        | BrokerError::ControlPlaneUnavailable
        | BrokerError::OutcomeRecorderThread(_)
        | BrokerError::SettingsCommitUnknown(_)
        | BrokerError::SettingsCommittedDegraded(_)
        | BrokerError::SessionAlreadyOpen => ReasonCode::InvalidMessage,
        BrokerError::ManualRequired => ReasonCode::ManualRequired,
        BrokerError::NoContext => ReasonCode::NoContext,
        BrokerError::NoPendingCommit | BrokerError::Stale => ReasonCode::Stale,
        BrokerError::NoSuggestion => ReasonCode::NoSuggestion,
        BrokerError::ProviderBusy => ReasonCode::ProviderError,
        BrokerError::UnknownSession => ReasonCode::UnknownSession,
    }
}

fn reason_for_frame(error: &FrameError) -> ReasonCode {
    match error {
        FrameError::Protocol(crate::protocol::ProtocolError::UnsupportedVersion(_)) => {
            ReasonCode::UnsupportedVersion
        }
        FrameError::Oversized(_) | FrameError::Empty | FrameError::Truncated => {
            ReasonCode::InvalidFrame
        }
        FrameError::Io(_)
        | FrameError::InvalidRuntimeDir
        | FrameError::MissingRuntimeDir
        | FrameError::PeerCredentialsUnavailable
        | FrameError::PeerUidMismatch
        | FrameError::Protocol(_)
        | FrameError::Serde(_)
        | FrameError::UnsafeSocket => ReasonCode::InvalidMessage,
    }
}

const fn reason_for_server(error: &ServerError) -> ReasonCode {
    match error {
        ServerError::InvalidCapability => ReasonCode::InvalidCapability,
        ServerError::Broker(error) => reason_for_broker(error),
        ServerError::Frame(FrameError::Protocol(
            crate::protocol::ProtocolError::UnsupportedVersion(_),
        )) => ReasonCode::UnsupportedVersion,
        ServerError::Frame(_) => ReasonCode::InvalidFrame,
        ServerError::ConnectionClosed
        | ServerError::HandshakeTimeout
        | ServerError::HelloRequired
        | ServerError::InvalidMessage
        | ServerError::InvalidSocketPath
        | ServerError::Io(_)
        | ServerError::Protocol(_)
        | ServerError::ResourceLimit
        | ServerError::SessionNotOwned
        | ServerError::SocketAlreadyActive
        | ServerError::UnsafeSocketPath => ReasonCode::InvalidMessage,
    }
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("broker")]
    Broker(#[from] BrokerError),
    #[error("connection_closed")]
    ConnectionClosed,
    #[error("frame")]
    Frame(#[from] FrameError),
    #[error("hello_required")]
    HelloRequired,
    #[error("handshake_timeout")]
    HandshakeTimeout,
    #[error("invalid_capability")]
    InvalidCapability,
    #[error("invalid_message")]
    InvalidMessage,
    #[error("invalid_socket_path")]
    InvalidSocketPath,
    #[error("io")]
    Io(#[from] io::Error),
    #[error("protocol")]
    Protocol(#[from] crate::protocol::ProtocolError),
    #[error("resource_limit")]
    ResourceLimit,
    #[error("session_not_owned")]
    SessionNotOwned,
    #[error("socket_already_active")]
    SocketAlreadyActive,
    #[error("unsafe_socket_path")]
    UnsafeSocketPath,
}

#[cfg(test)]
mod tests {
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
    use crate::engine::{Broker, BrokerConfig};
    use crate::ipc::{read_envelope, write_envelope};
    use crate::protocol::{
        Activation, AdapterDescriptor, AdapterKind, AuthorityAckPayload, AuthorityChangedPayload,
        Capability, ContextChangedPayload, Coordinates, EmptyPayload, FieldDescriptor,
        FieldPurpose, HelloPayload, MessageType, OffsetUnit, PROTOCOL_VERSION, Selection,
        SessionId, SessionOpenPayload, SuggestRequestPayload, TargetDescriptor, TargetKind,
        WireEnvelope,
    };
    use crate::provider::DeterministicPhraseProvider;

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
                min_v: PROTOCOL_VERSION,
                max_v: PROTOCOL_VERSION,
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
                min_v: PROTOCOL_VERSION,
                max_v: PROTOCOL_VERSION,
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
            Arc::new(DeterministicPhraseProvider::default()),
            BrokerConfig {
                debounce: Duration::from_millis(40),
                ..BrokerConfig::default()
            },
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
                unit: OffsetUnit::Utf16CodeUnits,
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
    async fn idle_deadline_closes_owned_sessions() {
        let (server, mut client) = UnixStream::pair().expect("Unix stream pair");
        let broker = broker();
        let task_broker = broker.clone();
        let connection = tokio::spawn(async move {
            serve_connection_with_timeouts(
                server,
                task_broker,
                CancellationToken::new(),
                Duration::from_secs(1),
                Duration::from_millis(50),
            )
            .await
        });
        open_test_session(&mut client, SessionId::new()).await;
        wait_for_sessions(&broker, 1).await;

        connection
            .await
            .expect("connection task")
            .expect("idle shutdown");
        assert_eq!(broker.session_count().await, 0);
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
}
