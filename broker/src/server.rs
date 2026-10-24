use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Semaphore, broadcast, mpsc};
use tokio::task::JoinSet;
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::control_plane::{ControlPlaneError, ControlPlaneSnapshot};
use crate::debug_trace;
use crate::engine::{
    Broker, BrokerError, BrokerEvent, BrokerEventSink, OutcomeRecorderHealth, SessionAuthority,
};
use crate::ipc::{FrameError, read_envelope, verify_peer_uid, write_envelope};
use crate::protocol::{
    AdapterKind, AuthorityAckPayload, AuthorityChangedPayload, CURRENT_PROTOCOL_VERSION,
    Capability, CommitResultPayload, ContextChangedPayload, ContextStatusPayload, ControlAction,
    ControlResultPayload, DesktopStatePayload, EditorFocusPayload, EmptyPayload, ErrorPayload,
    GlobalControlRequestPayload, HealthStatusPayload, HelloAckPayload, HelloPayload,
    MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_FRAME_BYTES, MAX_SAFE_COUNTER, MAX_SUGGESTION_CHARS,
    MAX_SUGGESTION_WORDS, MemoryStatusPayload, MessageType, PolicyQueryPayload,
    ProbeRequestPayload, ProtocolError, ReasonCode, RevisionRequestPayload, SessionClosePayload,
    SessionControlRequestPayload, SessionId, SessionOpenPayload, SettingsReplacePayload,
    SettingsStatusPayload, StyleKeyPayload, StyleSamplesPayload, StyleStatusPayload,
    SuggestCancelPayload, SuggestRequestPayload, WireEnvelope,
};
use crate::settings::{SETTINGS_SCHEMA, SettingsStoreError, SettingsV2};
use crate::style::{StyleKey, StyleKeyState};

const MAX_CONNECTIONS: usize = 32;
const MAX_SESSIONS_PER_CONNECTION: usize = 64;
const WIRE_QUEUE_CAPACITY: usize = 64;
const EVENT_QUEUE_CAPACITY: usize = 32;
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);
/// Closes quiet request/response clients. Policy subscribers (long-lived
/// adapters) are exempt: silence is their normal state, and a peer that exits
/// closes its socket.
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);

pub async fn run(socket_path: &Path, broker: Broker) -> Result<(), ServerError> {
    // Register both handlers before binding. Once the socket is visible, either
    // supported termination signal is therefore guaranteed to unwind through
    // this function and drop the inode-checked SocketGuard.
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let (listener, _guard) = bind_secure(socket_path)?;
    broker.remember_socket_path(socket_path.to_owned());
    let admissions = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let shutdown = CancellationToken::new();
    let mut connections = JoinSet::new();
    let provider_exited = broker.provider_exited();
    tokio::pin!(provider_exited);
    let mut accept_failing = false;
    let mut at_capacity = false;
    let outcome = loop {
        tokio::select! {
            () = &mut provider_exited => break Err(ServerError::ProviderExited),
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => {
                        accept_failing = false;
                        stream
                    }
                    Err(error) if is_transient_accept_error(&error) => {
                        if !accept_failing {
                            eprintln!("badi-broker: accept failed, retrying: {error}");
                        }
                        accept_failing = true;
                        time::sleep(ACCEPT_RETRY_DELAY).await;
                        continue;
                    }
                    Err(error) => break Err(ServerError::Io(error)),
                };
                let Ok(permit) = Arc::clone(&admissions).try_acquire_owned() else {
                    // Refused without a reply; say so once per run of refusals,
                    // so a client holding every slot is visible in the journal.
                    if !at_capacity {
                        eprintln!("badi-broker: {MAX_CONNECTIONS} connections open; refusing new ones");
                    }
                    at_capacity = true;
                    drop(stream);
                    continue;
                };
                at_capacity = false;
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

/// Failures of one accept call (descriptor or memory exhaustion, an aborted
/// peer, an interrupted call) rather than of the listening socket.
fn is_transient_accept_error(error: &io::Error) -> bool {
    use rustix::io::Errno;

    matches!(
        Errno::from_io_error(error),
        Some(
            Errno::MFILE
                | Errno::NFILE
                | Errno::CONNABORTED
                | Errno::INTR
                | Errno::AGAIN
                | Errno::NOBUFS
                | Errno::NOMEM
        )
    )
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

/// What a connection negotiated in its hello exchange.
struct Negotiated {
    authority: SessionAuthority,
    connection_id: String,
    policy_enabled: bool,
    authority_rx: broadcast::Receiver<AuthorityChangedPayload>,
}

/// Completes the hello exchange and, for a policy client, delivers the
/// current authority epoch. `None` means the peer closed before its hello
/// or the server began shutting down.
async fn negotiate(
    stream: &mut UnixStream,
    broker: &Broker,
    shutdown: &CancellationToken,
    hello_timeout: Duration,
) -> Result<Option<Negotiated>, ServerError> {
    if let Err(error) = verify_peer_uid(stream) {
        // A peer of another user gets no reply at all.
        let error = ServerError::from(error);
        log_refused_hello(&error);
        return Err(error);
    }
    let incoming = tokio::select! {
        () = shutdown.cancelled() => return Ok(None),
        incoming = time::timeout(hello_timeout, read_envelope(stream)) => incoming,
    };
    let first = match incoming {
        Ok(Ok(Some(first))) => first,
        Ok(Ok(None)) => return Ok(None),
        Ok(Err(error)) => {
            return Err(refuse_hello(stream, broker, None, error.into(), hello_timeout).await);
        }
        Err(_) => {
            let error = ServerError::HandshakeTimeout;
            return Err(refuse_hello(stream, broker, None, error, hello_timeout).await);
        }
    };
    let hello = match hello_terms(&first) {
        Ok(hello) => hello,
        Err(error) => {
            return Err(refuse_hello(stream, broker, first.id, error, hello_timeout).await);
        }
    };
    let negotiated = Negotiated {
        authority: SessionAuthority {
            adapter_kind: hello.adapter.kind,
            capabilities: hello.capabilities.clone(),
        },
        connection_id: format!("c:{}", uuid::Uuid::new_v4()),
        policy_enabled: hello.capabilities.contains(&Capability::Policy),
        authority_rx: broker.subscribe_authority_changes(),
    };
    let mut acknowledgment = WireEnvelope::global(
        MessageType::HelloAck,
        broker.mono_ms(),
        &HelloAckPayload {
            selected_v: CURRENT_PROTOCOL_VERSION,
            connection_id: negotiated.connection_id.clone(),
            enabled_capabilities: hello.capabilities,
            max_frame_bytes: MAX_FRAME_BYTES,
            max_before_chars: MAX_BEFORE_CHARS,
            max_after_chars: MAX_AFTER_CHARS,
            max_suggestion_chars: MAX_SUGGESTION_CHARS,
            max_suggestion_words: MAX_SUGGESTION_WORDS,
            paused: broker.is_paused().await,
        },
    )?;
    acknowledgment.id = first.id;
    if !write_handshake_frame(stream, &acknowledgment, shutdown, hello_timeout).await? {
        return Ok(None);
    }
    if negotiated.policy_enabled {
        broker
            .register_policy_client(negotiated.connection_id.clone())
            .await;
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
        if !write_handshake_frame(stream, &initial, shutdown, hello_timeout).await? {
            return Ok(None);
        }
    }
    Ok(Some(negotiated))
}

/// The hello, or why it is refused. Only the current version is spoken
/// (ADR 0007); its frame was already checked for that version.
fn hello_terms(first: &WireEnvelope) -> Result<HelloPayload, ServerError> {
    if first.message_type != MessageType::Hello {
        return Err(ServerError::HelloRequired);
    }
    let hello: HelloPayload = first.decode_payload()?;
    hello.validate()?;
    Ok(hello)
}

/// Tells the peer why its hello was refused, within the hello deadline, and
/// returns `error` for the connection to end with.
async fn refuse_hello(
    stream: &mut UnixStream,
    broker: &Broker,
    request_id: Option<String>,
    error: ServerError,
    hello_timeout: Duration,
) -> ServerError {
    log_refused_hello(&error);
    let refusal = WireEnvelope::global(
        MessageType::Error,
        broker.mono_ms(),
        &ErrorPayload::simple(reason_for_server(&error)),
    );
    if let Ok(mut refusal) = refusal {
        refusal.id = request_id;
        // A peer that left or stopped reading only loses the reason.
        let _ = time::timeout(hello_timeout, write_envelope(stream, &refusal)).await;
    }
    error
}

/// One content-free journal line per refused hello: the failure's class.
fn log_refused_hello(error: &ServerError) {
    let class = match error {
        ServerError::Protocol(error) | ServerError::Frame(FrameError::Protocol(error)) => {
            error.to_string()
        }
        ServerError::Frame(error) => error.to_string(),
        error => error.to_string(),
    };
    eprintln!("badi-broker: hello refused: {class}");
}

/// Returns `false` when shutdown interrupted the write.
async fn write_handshake_frame(
    stream: &mut UnixStream,
    envelope: &WireEnvelope,
    shutdown: &CancellationToken,
    hello_timeout: Duration,
) -> Result<bool, ServerError> {
    tokio::select! {
        () = shutdown.cancelled() => Ok(false),
        outgoing = time::timeout(hello_timeout, write_envelope(stream, envelope)) => {
            outgoing.map_err(|_| ServerError::HandshakeTimeout)??;
            Ok(true)
        }
    }
}

// Bounded forwarding and teardown deliberately stay together so every exit
// is visibly covered by the same owned-session cleanup path.
#[allow(clippy::too_many_lines)]
async fn serve_connection_with_timeouts(
    mut stream: UnixStream,
    broker: Broker,
    shutdown: CancellationToken,
    hello_timeout: Duration,
    idle_timeout: Duration,
) -> Result<(), ServerError> {
    let Some(Negotiated {
        authority,
        connection_id,
        policy_enabled,
        mut authority_rx,
    }) = negotiate(&mut stream, &broker, &shutdown, hello_timeout).await?
    else {
        return Ok(());
    };
    // A desktop helper reports only on change, so it stays connected while
    // idle like a policy subscriber; its report lasts as long as it does.
    let desktop_enabled = authority.capabilities.contains(&Capability::DesktopSignals);
    let stays_open = policy_enabled || desktop_enabled;

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
    let adapter_kind = authority.adapter_kind;
    let mut writer_task = tokio::spawn(async move {
        while let Some(envelope) = wire_rx.recv().await {
            debug_trace::wire("out", adapter_kind, &envelope);
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
            () = &mut idle_deadline, if !stays_open => break Ok(()),
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
                // Policy-capable adapters retire their bindings at every new
                // epoch. Retire both broker state and connection bookkeeping
                // before notifying them, including transient pause/resume.
                broker.close_owned_sessions(&owned_sessions).await;
                owned_sessions.clear();
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
                debug_trace::wire("in", adapter_kind, &envelope);
                let request_id = envelope.id.clone();
                let request_type = envelope.message_type;
                let mut handler = MessageHandler {
                    broker: &broker,
                    authority: &authority,
                    connection_id: &connection_id,
                    event_sink: &event_sink,
                    owned_sessions: &mut owned_sessions,
                    wire_tx: &wire_tx,
                };
                match handler.handle(envelope).await {
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
    if desktop_enabled {
        broker.forget_desktop_state(&connection_id).await;
    }
    if authority.capabilities.contains(&Capability::EditorFocus) {
        broker.forget_editor_focus(&connection_id).await;
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

/// Handles one inbound message within the connection's negotiated authority.
struct MessageHandler<'a> {
    broker: &'a Broker,
    authority: &'a SessionAuthority,
    connection_id: &'a str,
    event_sink: &'a BrokerEventSink,
    owned_sessions: &'a mut Vec<SessionId>,
    wire_tx: &'a mpsc::Sender<WireEnvelope>,
}

impl MessageHandler<'_> {
    async fn handle(&mut self, envelope: WireEnvelope) -> Result<(), ServerError> {
        match envelope.message_type {
            MessageType::SessionOpen => self.open_session(envelope).await,
            MessageType::SessionClose => self.close_session(envelope).await,
            MessageType::ContextChanged => self.update_context(envelope).await,
            MessageType::SuggestRequest => self.request_suggestion(envelope).await,
            MessageType::SuggestCancel => self.cancel_suggestion(envelope).await,
            MessageType::RevisionRequest => self.request_revision(envelope).await,
            MessageType::ControlRequest => self.control(envelope).await,
            MessageType::CommitResult => self.commit_result(envelope).await,
            MessageType::HealthRequest => self.health(envelope).await,
            MessageType::PolicyQuery => self.resolve_policy(envelope).await,
            MessageType::AuthorityAck => self.acknowledge_authority(envelope).await,
            MessageType::SettingsGet => self.settings_get(envelope).await,
            MessageType::SettingsReplace => self.settings_replace(envelope).await,
            MessageType::MemoryClear => self.memory_clear(envelope).await,
            MessageType::ProbeRequest => self.probe(envelope).await,
            MessageType::DesktopState => self.desktop_state(envelope).await,
            MessageType::EditorFocus => self.editor_focus(envelope).await,
            MessageType::StyleKey => self.style_key(envelope).await,
            MessageType::StyleQuery => self.style_query(envelope).await,
            MessageType::StyleForget => self.style_forget(envelope).await,
            MessageType::StyleExport => self.style_export(envelope).await,
            MessageType::ContextQuery => self.context_query(envelope).await,
            MessageType::ContextForget => self.context_forget(envelope).await,
            MessageType::StatsQuery => self.stats_query(envelope).await,
            MessageType::StatsForget => self.stats_forget(envelope).await,
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
            | MessageType::ProbeResult
            | MessageType::DesktopStatus
            | MessageType::EditorStatus
            | MessageType::StyleStatus
            | MessageType::StyleSamples
            | MessageType::ContextStatus
            | MessageType::StatsStatus
            | MessageType::RevisionShow
            | MessageType::RevisionClear
            | MessageType::Error => Err(ServerError::InvalidMessage),
        }
    }

    fn reply<P: Serialize>(
        &self,
        message_type: MessageType,
        request_id: Option<String>,
        payload: &P,
    ) -> Result<(), ServerError> {
        send_reply(self.wire_tx, self.broker, message_type, request_id, payload)
    }

    async fn open_session(&mut self, envelope: WireEnvelope) -> Result<(), ServerError> {
        self.broker
            .require_current_authority_ack(self.connection_id)
            .await?;
        require_capability(self.authority, Capability::Context)?;
        require_capability(self.authority, Capability::Suggestion)?;
        require_capability(self.authority, Capability::Policy)?;
        ensure_session_capacity(self.owned_sessions)?;
        let coordinates = envelope.coordinates()?;
        let payload: SessionOpenPayload = envelope.decode_payload()?;
        payload.target.validate_wire()?;
        self.broker
            .open_session(
                coordinates,
                payload,
                self.authority.clone(),
                self.event_sink.clone(),
            )
            .await?;
        self.owned_sessions.push(coordinates.session_id);
        Ok(())
    }

    async fn close_session(&mut self, envelope: WireEnvelope) -> Result<(), ServerError> {
        let coordinates = envelope.coordinates()?;
        ensure_owned(self.owned_sessions, coordinates.session_id)?;
        let _: SessionClosePayload = envelope.decode_payload()?;
        self.broker.close_session(coordinates).await?;
        self.owned_sessions
            .retain(|session_id| *session_id != coordinates.session_id);
        Ok(())
    }

    async fn update_context(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        self.broker
            .require_current_authority_ack(self.connection_id)
            .await?;
        require_capability(self.authority, Capability::Context)?;
        let coordinates = envelope.coordinates()?;
        ensure_owned(self.owned_sessions, coordinates.session_id)?;
        let payload: ContextChangedPayload = envelope.decode_payload()?;
        payload.validate()?;
        let _ = self.broker.update_context(coordinates, payload).await?;
        Ok(())
    }

    async fn request_suggestion(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        self.broker
            .require_current_authority_ack(self.connection_id)
            .await?;
        require_capability(self.authority, Capability::Suggestion)?;
        let coordinates = envelope.coordinates()?;
        ensure_owned(self.owned_sessions, coordinates.session_id)?;
        let payload: SuggestRequestPayload = envelope.decode_payload()?;
        self.broker
            .request_suggestion(coordinates, payload, envelope.id)
            .await?;
        Ok(())
    }

    async fn request_revision(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        self.broker
            .require_current_authority_ack(self.connection_id)
            .await?;
        require_capability(self.authority, Capability::SelectionRevision)?;
        let coordinates = envelope.coordinates()?;
        ensure_owned(self.owned_sessions, coordinates.session_id)?;
        let payload: RevisionRequestPayload = envelope.decode_payload()?;
        self.broker
            .request_revision(coordinates, payload, envelope.id)
            .await?;
        Ok(())
    }

    async fn cancel_suggestion(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::Suggestion)?;
        let coordinates = envelope.coordinates()?;
        ensure_owned(self.owned_sessions, coordinates.session_id)?;
        let payload: SuggestCancelPayload = envelope.decode_payload()?;
        self.broker.cancel_suggestion(coordinates, payload).await?;
        Ok(())
    }

    async fn control(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::Control)?;
        let action: ControlAction = serde_json::from_value(
            envelope
                .payload
                .get("action")
                .cloned()
                .ok_or(ServerError::InvalidMessage)?,
        )
        .map_err(|_| ServerError::InvalidMessage)?;
        if action.is_global() {
            let payload: GlobalControlRequestPayload = envelope.decode_payload()?;
            payload.validate()?;
            let mut snooze = None;
            let mut reason = ReasonCode::Accepted;
            let paused = match action {
                ControlAction::Pause => self.broker.set_paused(true).await,
                ControlAction::Resume => self.broker.set_paused(false).await,
                ControlAction::PauseToggle => self.broker.toggle_paused().await,
                ControlAction::Snooze => {
                    let minutes = payload.minutes.ok_or(ServerError::InvalidMessage)?;
                    let duration = Duration::from_secs(u64::from(minutes) * 60);
                    match self.broker.snooze_focused(duration).await {
                        Ok(status) => snooze = Some(status),
                        Err(refusal) => reason = refusal,
                    }
                    self.broker.is_paused().await
                }
                ControlAction::SnoozeEnd => {
                    self.broker.end_snoozes().await;
                    self.broker.is_paused().await
                }
                ControlAction::Request
                | ControlAction::AcceptWord
                | ControlAction::AcceptAll
                | ControlAction::Dismiss => return Err(ServerError::InvalidMessage),
            };
            let result = ControlResultPayload {
                action,
                accepted: reason == ReasonCode::Accepted,
                reason,
                paused,
                snooze,
            };
            return self.reply(MessageType::ControlResult, envelope.id, &result);
        }
        // A session action may request a suggestion or grant a commit, so
        // it waits for the current epoch like the data plane. A connection
        // without policy (`badictl`) is not gated.
        self.broker
            .require_current_authority_ack(self.connection_id)
            .await?;
        let coordinates = envelope.coordinates()?;
        let payload: SessionControlRequestPayload = envelope.decode_payload()?;
        self.broker
            .session_control(coordinates, payload, envelope.id.clone())
            .await?;
        let result = ControlResultPayload {
            action,
            accepted: true,
            reason: ReasonCode::Accepted,
            paused: self.broker.is_paused().await,
            snooze: None,
        };
        self.reply(MessageType::ControlResult, envelope.id, &result)
    }

    async fn commit_result(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        let coordinates = envelope.coordinates()?;
        ensure_owned(self.owned_sessions, coordinates.session_id)?;
        let payload: CommitResultPayload = envelope.decode_payload()?;
        self.broker.commit_result(coordinates, payload).await?;
        Ok(())
    }

    async fn health(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::Health)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        let health = self.broker.health_snapshot().await;
        let status = HealthStatusPayload {
            provider: health.provider,
            paused: health.paused,
            authority_epoch: health.authority_epoch,
            settings_revision: health.settings_revision,
            control_plane_degraded: health.control_plane_degraded,
            sessions: health.sessions,
            socket_mode: self.broker.measured_socket_mode(),
            max_frame_bytes: health.max_frame_bytes,
            metrics: health.metrics,
            active: health.active,
            desktop: Some(health.desktop),
            snoozes: health.snoozes,
        };
        self.reply(MessageType::HealthStatus, envelope.id, &status)
    }

    async fn style_key(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::StyleKey)?;
        let payload: StyleKeyPayload = envelope.decode_payload()?;
        let key = StyleKey::from_hex(&payload.key_id, &payload.key)
            .map_err(|_| ProtocolError::InvalidPayload)?;
        self.broker.deliver_style_key(key)?;
        self.reply_style_status(envelope.id).await
    }

    async fn style_query(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        self.reply_style_status(envelope.id).await
    }

    async fn style_forget(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        self.broker.forget_style().await?;
        self.reply_style_status(envelope.id).await
    }

    async fn style_export(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        let samples = self.broker.export_style().await?;
        self.reply(
            MessageType::StyleSamples,
            envelope.id,
            &StyleSamplesPayload { samples },
        )
    }

    async fn reply_style_status(&self, request_id: Option<String>) -> Result<(), ServerError> {
        let enabled = self
            .broker
            .control_plane_snapshot()
            .await?
            .settings
            .style_learning;
        let status = self.broker.style_status();
        let payload = StyleStatusPayload {
            enabled,
            available: status.is_some(),
            key: status.map_or(StyleKeyState::Absent, |status| status.key),
            samples: status.map_or(0, |status| status.samples),
            subjects: status.map_or(0, |status| status.subjects),
        };
        self.reply(MessageType::StyleStatus, request_id, &payload)
    }

    async fn context_query(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        self.reply_context_status(envelope.id).await
    }

    async fn context_forget(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        self.broker.forget_learned().await?;
        self.reply_context_status(envelope.id).await
    }

    async fn reply_context_status(&self, request_id: Option<String>) -> Result<(), ServerError> {
        let chosen = self
            .broker
            .control_plane_snapshot()
            .await?
            .settings
            .context_words;
        let status = self.broker.learned_status();
        let payload = ContextStatusPayload {
            enabled: chosen == Some(true),
            chosen: chosen.is_some(),
            available: status.is_some(),
            key: status.map_or(StyleKeyState::Absent, |status| status.key),
            subjects: status.map_or(0, |status| status.subjects),
            words: status.and_then(|status| status.words),
        };
        self.reply(MessageType::ContextStatus, request_id, &payload)
    }

    async fn stats_query(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        let status = self.broker.stats_status().await?;
        self.reply(MessageType::StatsStatus, envelope.id, &status)
    }

    async fn stats_forget(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        let status = self.broker.forget_stats().await?;
        self.reply(MessageType::StatsStatus, envelope.id, &status)
    }

    async fn desktop_state(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::DesktopSignals)?;
        let report: DesktopStatePayload = envelope.decode_payload()?;
        let status = self
            .broker
            .report_desktop_state(self.connection_id, report)
            .await?;
        self.reply(MessageType::DesktopStatus, envelope.id, &status)
    }

    async fn editor_focus(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::EditorFocus)?;
        let report: EditorFocusPayload = envelope.decode_payload()?;
        let status = self
            .broker
            .report_editor_focus(self.connection_id, report.focused)
            .await;
        self.reply(MessageType::EditorStatus, envelope.id, &status)
    }

    async fn resolve_policy(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::Policy)?;
        let payload: PolicyQueryPayload = envelope.decode_payload()?;
        payload.validate()?;
        let status = self.broker.resolve_policy(&payload.target).await;
        self.reply(MessageType::PolicyStatus, envelope.id, &status)
    }

    async fn acknowledge_authority(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_capability(self.authority, Capability::Policy)?;
        let payload: AuthorityAckPayload = envelope.decode_payload()?;
        payload.validate()?;
        self.broker
            .acknowledge_authority(self.connection_id, payload.authority_epoch)
            .await?;
        Ok(())
    }

    async fn settings_get(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        let snapshot = self.broker.control_plane_snapshot().await?;
        self.reply_settings_status(envelope.id, &snapshot)
    }

    async fn settings_replace(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let payload: SettingsReplacePayload = envelope.decode_payload()?;
        payload.validate()?;
        let source_schema = payload
            .document
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .ok_or(ServerError::InvalidMessage)?;
        // Only `badi.settings.v2` crosses the wire; a v1 file migrates on disk.
        if source_schema != SETTINGS_SCHEMA {
            return Err(ServerError::InvalidMessage);
        }
        let next: SettingsV2 =
            serde_json::from_value(payload.document).map_err(|_| ServerError::InvalidMessage)?;
        next.validate().map_err(|_| ServerError::InvalidMessage)?;
        let snapshot = self
            .broker
            .replace_settings(payload.expected_revision, next)
            .await?;
        self.reply_settings_status(envelope.id, &snapshot)
    }

    fn reply_settings_status(
        &self,
        request_id: Option<String>,
        snapshot: &ControlPlaneSnapshot,
    ) -> Result<(), ServerError> {
        let status = settings_status_payload(snapshot, self.broker.outcome_recorder_health())?;
        self.reply(MessageType::SettingsStatus, request_id, &status)
    }

    async fn memory_clear(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let _: EmptyPayload = envelope.decode_payload()?;
        let (changed, snapshot) = self.broker.clear_personalization().await?;
        let status = MemoryStatusPayload {
            revision: snapshot.personalization.revision,
            records: saturating_counter(snapshot.personalization.records.len()),
            bytes: saturating_counter(snapshot.persisted_personalization_bytes),
            changed,
        };
        self.reply(MessageType::MemoryStatus, envelope.id, &status)
    }

    async fn probe(&self, envelope: WireEnvelope) -> Result<(), ServerError> {
        require_settings_authority(self.authority)?;
        let payload: ProbeRequestPayload = envelope.decode_payload()?;
        let result = self.broker.probe(payload).await?;
        result.validate()?;
        self.reply(MessageType::ProbeResult, envelope.id, &result)
    }
}

fn require_settings_authority(authority: &SessionAuthority) -> Result<(), ServerError> {
    require_capability(authority, Capability::Settings)?;
    if authority.adapter_kind == AdapterKind::Cli {
        Ok(())
    } else {
        Err(ServerError::InvalidCapability)
    }
}

fn settings_status_payload(
    snapshot: &ControlPlaneSnapshot,
    recorder: OutcomeRecorderHealth,
) -> Result<SettingsStatusPayload, ServerError> {
    let payload = SettingsStatusPayload {
        document: serde_json::to_value(&snapshot.settings).map_err(ProtocolError::from)?,
        personalization_revision: snapshot.personalization.revision,
        personalization_records: saturating_counter(snapshot.personalization.records.len()),
        personalization_bytes: saturating_counter(snapshot.persisted_personalization_bytes),
        personalization_store_available: snapshot.personalization_store_available,
        personalization_recorder_available: recorder.available,
        personalization_write_failures: recorder.write_failures.min(MAX_SAFE_COUNTER),
        personalization_dropped_signals: recorder.dropped_signals.min(MAX_SAFE_COUNTER),
    };
    payload.validate()?;
    Ok(payload)
}

fn saturating_counter(value: usize) -> u64 {
    u64::try_from(value)
        .unwrap_or(MAX_SAFE_COUNTER)
        .min(MAX_SAFE_COUNTER)
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

fn send_reply<P: Serialize>(
    wire_tx: &mpsc::Sender<WireEnvelope>,
    broker: &Broker,
    message_type: MessageType,
    request_id: Option<String>,
    payload: &P,
) -> Result<(), ServerError> {
    let mut envelope = WireEnvelope::global(message_type, broker.mono_ms(), payload)?;
    envelope.id = request_id;
    wire_tx
        .try_send(envelope)
        .map_err(|_| ServerError::ConnectionClosed)
}

fn send_error(
    wire_tx: &mpsc::Sender<WireEnvelope>,
    broker: &Broker,
    request_id: Option<String>,
    reason: ReasonCode,
) -> Result<(), ServerError> {
    send_reply(
        wire_tx,
        broker,
        MessageType::Error,
        request_id,
        &ErrorPayload::simple(reason),
    )
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
        if let Some((code, committed)) = settings_replace_failure(error) {
            payload.code = code;
            payload.committed = committed;
        }
    }
    payload.validate()?;
    send_reply(wire_tx, broker, MessageType::Error, request_id, &payload)
}

/// The settings-specific code, and whether the replacement is known to have
/// committed (`None` when unknown).
fn settings_replace_failure(error: &BrokerError) -> Option<(ReasonCode, Option<bool>)> {
    match error {
        BrokerError::SettingsCommittedDegraded(_) => {
            Some((ReasonCode::SettingsCommittedDegraded, Some(true)))
        }
        BrokerError::SettingsCommitUnknown(_) | BrokerError::ControlPlaneTask => {
            Some((ReasonCode::SettingsCommitUnknown, None))
        }
        BrokerError::ControlPlane(ControlPlaneError::Settings(
            SettingsStoreError::RevisionConflict { .. },
        )) => Some((ReasonCode::SettingsConflict, Some(false))),
        BrokerError::ControlPlane(_) | BrokerError::ControlPlaneUnavailable => {
            Some((ReasonCode::SettingsRejected, Some(false)))
        }
        _ => None,
    }
}

const fn reason_for_broker(error: &BrokerError) -> ReasonCode {
    match error {
        BrokerError::Denied(reason) => reason.refusal_code(),
        BrokerError::InvalidCapability => ReasonCode::InvalidCapability,
        BrokerError::CommitLeaseExpired => ReasonCode::Expired,
        BrokerError::EventSinkClosed | BrokerError::ShuttingDown => ReasonCode::SessionClosed,
        BrokerError::InvalidPayload
        | BrokerError::Protocol(_)
        | BrokerError::ControlPlane(_)
        | BrokerError::ControlPlaneTask
        | BrokerError::ControlPlaneTimeout
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
        FrameError::Protocol(ProtocolError::UnsupportedVersion(_)) => {
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
        ServerError::ProviderExited => ReasonCode::ProviderError,
        ServerError::Broker(error) => reason_for_broker(error),
        ServerError::Frame(FrameError::Protocol(ProtocolError::UnsupportedVersion(_)))
        | ServerError::Protocol(
            ProtocolError::UnsupportedVersion(_) | ProtocolError::VersionNegotiationFailed,
        ) => ReasonCode::UnsupportedVersion,
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
    Protocol(#[from] ProtocolError),
    #[error("provider_exited")]
    ProviderExited,
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
mod tests;
