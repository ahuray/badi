use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::engine::{Broker, BrokerError, BrokerEvent, SessionAuthority};
use crate::ipc::{FrameError, read_envelope, verify_peer_uid, write_envelope};
use crate::policy::PolicyReason;
use crate::protocol::{
    Capability, CommitResultPayload, ContextChangedPayload, ControlAction, ControlResultPayload,
    EmptyPayload, ErrorPayload, GlobalControlRequestPayload, HealthStatusPayload, HelloAckPayload,
    HelloPayload, MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_FRAME_BYTES, MAX_SUGGESTION_CHARS,
    MAX_SUGGESTION_WORDS, MessageType, PROTOCOL_VERSION, ReasonCode, SessionClosePayload,
    SessionControlRequestPayload, SessionId, SessionOpenPayload, SuggestCancelPayload,
    SuggestRequestPayload, WireEnvelope,
};

pub async fn run(socket_path: &Path, broker: Broker) -> Result<(), ServerError> {
    let (listener, _guard) = bind_secure(socket_path)?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let broker = broker.clone();
                tokio::spawn(async move {
                    let _ = serve_connection(stream, broker).await;
                });
            }
            signal = tokio::signal::ctrl_c() => {
                signal?;
                return Ok(());
            }
        }
    }
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

async fn serve_connection(stream: UnixStream, broker: Broker) -> Result<(), ServerError> {
    verify_peer_uid(&stream)?;
    let (mut reader, mut writer) = stream.into_split();
    let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<WireEnvelope>();
    let writer_task = tokio::spawn(async move {
        while let Some(envelope) = wire_rx.recv().await {
            write_envelope(&mut writer, &envelope).await?;
        }
        Ok::<(), FrameError>(())
    });

    let first = read_envelope(&mut reader)
        .await?
        .ok_or(ServerError::HelloRequired)?;
    if first.message_type != MessageType::Hello {
        return Err(ServerError::HelloRequired);
    }
    let hello: HelloPayload = first.decode_payload()?;
    hello.validate()?;
    let authority = SessionAuthority {
        adapter_kind: hello.adapter.kind,
        capabilities: hello.capabilities.clone(),
    };
    let mut acknowledgment = WireEnvelope::global(
        MessageType::HelloAck,
        broker.mono_ms(),
        &HelloAckPayload {
            selected_v: PROTOCOL_VERSION,
            connection_id: format!("c:{}", uuid::Uuid::new_v4()),
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
    wire_tx
        .send(acknowledgment)
        .map_err(|_| ServerError::ConnectionClosed)?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<BrokerEvent>();
    let event_wire_tx = wire_tx.clone();
    let event_broker = broker.clone();
    let event_task = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let envelope = event.into_wire(event_broker.mono_ms())?;
            event_wire_tx
                .send(envelope)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        Ok::<(), ServerError>(())
    });

    let mut owned_sessions = Vec::<SessionId>::new();
    loop {
        let envelope = match read_envelope(&mut reader).await {
            Ok(Some(envelope)) => envelope,
            Ok(None) => break,
            Err(error) => {
                let _ = send_error(&wire_tx, &broker, None, reason_for_frame(&error));
                break;
            }
        };
        let request_id = envelope.id.clone();
        match handle_message(
            &broker,
            &authority,
            &event_tx,
            &mut owned_sessions,
            envelope,
            &wire_tx,
        )
        .await
        {
            Ok(()) => {}
            Err(ServerError::Broker(error)) => {
                let _ = send_error(&wire_tx, &broker, request_id, reason_for_broker(&error));
            }
            Err(error) => {
                let _ = send_error(&wire_tx, &broker, request_id, reason_for_server(&error));
                break;
            }
        }
    }

    broker.close_owned_sessions(&owned_sessions).await;
    event_task.abort();
    drop(wire_tx);
    let _ = writer_task.await;
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn handle_message(
    broker: &Broker,
    authority: &SessionAuthority,
    event_tx: &mpsc::UnboundedSender<BrokerEvent>,
    owned_sessions: &mut Vec<SessionId>,
    envelope: WireEnvelope,
    wire_tx: &mpsc::UnboundedSender<WireEnvelope>,
) -> Result<(), ServerError> {
    match envelope.message_type {
        MessageType::SessionOpen => {
            require_capability(authority, Capability::Context)?;
            require_capability(authority, Capability::Suggestion)?;
            let coordinates = envelope.coordinates()?;
            let payload: SessionOpenPayload = envelope.decode_payload()?;
            broker
                .open_session(coordinates, payload, authority.clone(), event_tx.clone())
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
                .send(result)
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
                    sessions: health.sessions,
                    socket_mode: "0600".to_owned(),
                    max_frame_bytes: health.max_frame_bytes,
                    metrics: health.metrics,
                    active: health.active,
                },
            )?;
            response.id = envelope.id;
            wire_tx
                .send(response)
                .map_err(|_| ServerError::ConnectionClosed)?;
        }
        MessageType::Hello
        | MessageType::HelloAck
        | MessageType::SuggestionShow
        | MessageType::SuggestionClear
        | MessageType::ControlResult
        | MessageType::CommitPrepare
        | MessageType::HealthStatus
        | MessageType::Error => return Err(ServerError::InvalidMessage),
    }
    Ok(())
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

fn send_error(
    wire_tx: &mpsc::UnboundedSender<WireEnvelope>,
    broker: &Broker,
    request_id: Option<String>,
    reason: ReasonCode,
) -> Result<(), ServerError> {
    let mut envelope = WireEnvelope::global(
        MessageType::Error,
        broker.mono_ms(),
        &ErrorPayload { code: reason },
    )?;
    envelope.id = request_id;
    wire_tx
        .send(envelope)
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
        BrokerError::EventSinkClosed => ReasonCode::SessionClosed,
        BrokerError::Denied(
            PolicyReason::AllowedAlways
            | PolicyReason::AllowedExplicit
            | PolicyReason::ManualRequired,
        )
        | BrokerError::InvalidPayload
        | BrokerError::Protocol(_)
        | BrokerError::SessionAlreadyOpen => ReasonCode::InvalidMessage,
        BrokerError::ManualRequired => ReasonCode::ManualRequired,
        BrokerError::NoContext => ReasonCode::NoContext,
        BrokerError::NoPendingCommit | BrokerError::Stale => ReasonCode::Stale,
        BrokerError::NoSuggestion => ReasonCode::NoSuggestion,
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
        | ServerError::HelloRequired
        | ServerError::InvalidMessage
        | ServerError::InvalidSocketPath
        | ServerError::Io(_)
        | ServerError::Protocol(_)
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

    use tempfile::tempdir;

    use super::bind_secure;

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
