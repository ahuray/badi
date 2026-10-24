use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use badi_broker::ipc::{
    default_socket_path, read_envelope, verify_peer_uid, verify_socket_metadata, write_envelope,
};
use badi_broker::metrics::MetricsSnapshot;
use badi_broker::model_selection::{ModelAdvice, ModelUseCase, detect_hardware, recommend_model};
use badi_broker::protocol::{
    ActiveLocator, AdapterDescriptor, AdapterKind, CURRENT_PROTOCOL_VERSION, Capability,
    ContextStatusPayload, ControlAction, ControlResultPayload, Coordinates, DesktopStatusPayload,
    ErrorPayload, GlobalControlRequestPayload, HealthStatusPayload, HelloAckPayload, HelloPayload,
    MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_SAFE_COUNTER, MAX_SNOOZE_MINUTES, MemoryStatusPayload,
    MessageType, ProbeRequestPayload, ProbeResultPayload, ProtocolError, ProviderKind, ReasonCode,
    SessionControlRequestPayload, SettingsReplacePayload, SettingsStatusPayload, SnoozeStatus,
    StatsStatusPayload, StyleKeyPayload, StyleSamplesPayload, StyleStatusPayload, WireEnvelope,
};
use badi_broker::settings::{
    MAX_SETTINGS_BYTES, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA, SettingsV2,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::UnixStream;

const CLI_CAPABILITIES: [Capability; 4] = [
    Capability::Control,
    Capability::Health,
    Capability::Settings,
    Capability::StyleKey,
];
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);
const OVERVIEW_SNAPSHOT_ATTEMPTS: usize = 3;
/// `snooze` without MINUTES, as Cotypist's snooze shortcut.
const DEFAULT_SNOOZE_MINUTES: u16 = 10;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let result = run().await;
    if let Err(error) = result {
        eprintln!("error_code={error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), CliError> {
    match parse_arguments(std::env::args().skip(1))? {
        ParsedCommand::Help => print!("{CLI_USAGE}"),
        ParsedCommand::Version => {
            println!("{}", badi_broker::build_info::version_line("badictl"));
        }
        ParsedCommand::Local(command) => println!("{}", local_report(command)?),
        ParsedCommand::Remote {
            socket_path,
            command,
        } => {
            // Read standard input before connecting, so typing it cannot
            // outlast the handshake timeout.
            let command = with_stdin(command).await?;
            let mut stream = connect(&socket_path).await?;
            println!("{}", broker_report(&mut stream, command).await?);
        }
    }
    Ok(())
}

fn local_report(command: LocalCommand) -> Result<String, CliError> {
    let hardware = detect_hardware();
    Ok(match command {
        LocalCommand::Hardware => serde_json::to_string_pretty(&hardware)?,
        LocalCommand::Models(use_case) => {
            serde_json::to_string_pretty(&recommend_model(hardware, use_case))?
        }
    })
}

async fn broker_report(stream: &mut UnixStream, command: Command) -> Result<String, CliError> {
    Ok(match command {
        Command::Status => {
            let status = request_health(stream).await?;
            serde_json::to_string(&redact_health_status(&status))?
        }
        Command::Overview => {
            serde_json::to_string_pretty(&request_coherent_overview(stream).await?)?
        }
        Command::SettingsShow => {
            serde_json::to_string_pretty(&request_settings(stream).await?.document)?
        }
        Command::SettingsReplace {
            expected_revision,
            document,
        } => {
            let payload = SettingsReplacePayload {
                expected_revision,
                document,
            };
            let request = cli_global(MessageType::SettingsReplace, &payload)?;
            let status = exchange(stream, request, validate_settings_response).await?;
            serde_json::to_string_pretty(&status.document)?
        }
        Command::SettingsReplaceStdin { .. } | Command::Style(StyleCommand::Key(None)) => {
            return Err(CliError::Arguments);
        }
        Command::MemoryClear => {
            let request = cli_global(MessageType::MemoryClear, &serde_json::json!({}))?;
            let memory = exchange(stream, request, validate_memory_response).await?;
            serde_json::to_string(&memory)?
        }
        Command::Global(payload) => {
            let request = cli_global(MessageType::ControlRequest, &payload)?;
            request_control(stream, request, payload.action).await?
        }
        Command::Session(action) => {
            let status = request_health(stream).await?;
            let active = status.active.ok_or(CliError::NoActiveSession)?;
            request_control(stream, addressed_control(action, active)?, action).await?
        }
        Command::Probe { payload, .. } => {
            serde_json::to_string(&request_probe(stream, &payload).await?)?
        }
        Command::Style(StyleCommand::Status) => {
            let request = cli_global(MessageType::StyleQuery, &serde_json::json!({}))?;
            serde_json::to_string(&exchange(stream, request, validate_style_status).await?)?
        }
        Command::Style(StyleCommand::Forget) => {
            let request = cli_global(MessageType::StyleForget, &serde_json::json!({}))?;
            serde_json::to_string(&exchange(stream, request, validate_style_status).await?)?
        }
        Command::Style(StyleCommand::Export) => {
            let request = cli_global(MessageType::StyleExport, &serde_json::json!({}))?;
            serde_json::to_string_pretty(&exchange(stream, request, validate_style_samples).await?)?
        }
        Command::Context(command) => {
            let message = match command {
                ContextCommand::Status => MessageType::ContextQuery,
                ContextCommand::Forget => MessageType::ContextForget,
            };
            let request = cli_global(message, &serde_json::json!({}))?;
            serde_json::to_string(&exchange(stream, request, validate_context_status).await?)?
        }
        Command::Stats(command) => {
            let message = match command {
                StatsCommand::Status => MessageType::StatsQuery,
                StatsCommand::Forget => MessageType::StatsForget,
            };
            let request = cli_global(message, &serde_json::json!({}))?;
            serde_json::to_string(&exchange(stream, request, validate_stats_status).await?)?
        }
        Command::Style(StyleCommand::Key(Some(payload))) => {
            let request = cli_global(MessageType::StyleKey, &payload)?;
            serde_json::to_string(&exchange(stream, request, validate_style_status).await?)?
        }
    })
}

fn validate_style_status(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<StyleStatusPayload, CliError> {
    decode_reply(response, request_id, MessageType::StyleStatus, |_| Ok(()))
}

fn validate_context_status(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<ContextStatusPayload, CliError> {
    decode_reply(response, request_id, MessageType::ContextStatus, |_| Ok(()))
}

fn validate_stats_status(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<StatsStatusPayload, CliError> {
    decode_reply(response, request_id, MessageType::StatsStatus, |payload| {
        let summary = &payload.summary;
        if summary.days.len() <= usize::from(badi_broker::stats::STATS_RETENTION_DAYS) + 1
            && summary.subjects.len() <= 3 * badi_broker::stats::MAX_SUMMARY_SUBJECTS
        {
            Ok(())
        } else {
            Err(ProtocolError::InvalidPayload)
        }
    })
}

fn validate_style_samples(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<StyleSamplesPayload, CliError> {
    decode_reply(response, request_id, MessageType::StyleSamples, |payload| {
        if payload.samples.len() <= badi_broker::style::MAX_STYLE_SAMPLES {
            Ok(())
        } else {
            Err(ProtocolError::InvalidPayload)
        }
    })
}

/// Reads `KEY_ID KEY` (lowercase hex) from stdin, never from argv.
async fn read_style_key<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
) -> Result<StyleKeyPayload, CliError> {
    use tokio::io::AsyncReadExt as _;
    let mut bytes = Vec::new();
    reader.take(256).read_to_end(&mut bytes).await?;
    let text = std::str::from_utf8(&bytes).map_err(|_| CliError::Arguments)?;
    let mut parts = text.split_whitespace();
    let (Some(key_id), Some(key), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(CliError::Arguments);
    };
    badi_broker::style::StyleKey::from_hex(key_id, key).map_err(|_| CliError::Arguments)?;
    Ok(StyleKeyPayload {
        key_id: key_id.to_owned(),
        key: key.to_owned(),
    })
}

async fn with_stdin(command: Command) -> Result<Command, CliError> {
    if let Command::SettingsReplaceStdin { expected_revision } = command {
        return read_settings_stdin(tokio::io::stdin(), expected_revision).await;
    }
    if command == Command::Style(StyleCommand::Key(None)) {
        let payload = read_style_key(tokio::io::stdin()).await?;
        return Ok(Command::Style(StyleCommand::Key(Some(payload))));
    }
    let Command::Probe {
        mut payload,
        text_from_stdin: true,
    } = command
    else {
        return Ok(command);
    };
    payload.before = read_probe_text(tokio::io::stdin()).await?;
    payload.validate().map_err(|_| CliError::Arguments)?;
    Ok(Command::Probe {
        payload,
        text_from_stdin: false,
    })
}

async fn read_settings_stdin<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    expected_revision: u64,
) -> Result<Command, CliError> {
    use tokio::io::AsyncReadExt as _;
    let mut bytes = Vec::new();
    reader
        .take((MAX_SETTINGS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(CliError::Arguments);
    }
    settings_replacement(
        expected_revision,
        std::str::from_utf8(&bytes).map_err(|_| CliError::Arguments)?,
    )
}

/// Reads probe text without placing it in argv (visible to other local users
/// in the process list) or shell history. One trailing newline is removed.
async fn read_probe_text<R: tokio::io::AsyncRead + Unpin>(reader: R) -> Result<String, CliError> {
    use tokio::io::AsyncReadExt as _;
    // At most four UTF-8 bytes per character plus a CRLF terminator.
    const LIMIT: usize = MAX_BEFORE_CHARS * 4 + 2;
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(LIMIT + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > LIMIT {
        return Err(CliError::Arguments);
    }
    let mut text = String::from_utf8(bytes).map_err(|_| CliError::Arguments)?;
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    Ok(text)
}

async fn request_probe(
    stream: &mut UnixStream,
    payload: &ProbeRequestPayload,
) -> Result<ProbeReport, CliError> {
    let request = cli_global(MessageType::ProbeRequest, payload)?;
    let started = Instant::now();
    let result = exchange(stream, request, validate_probe_response).await?;
    Ok(ProbeReport {
        result,
        round_trip_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// The broker's probe result with the client-observed round trip.
#[derive(Debug, Serialize)]
struct ProbeReport {
    #[serde(flatten)]
    result: ProbeResultPayload,
    round_trip_ms: u64,
}

fn validate_probe_response(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<ProbeResultPayload, CliError> {
    decode_reply(
        response,
        request_id,
        MessageType::ProbeResult,
        ProbeResultPayload::validate,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RedactedActiveStatus {
    present: bool,
    has_suggestion: bool,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct RedactedHealthStatus<'a> {
    provider: ProviderKind,
    paused: bool,
    authority_epoch: u64,
    settings_revision: u64,
    control_plane_degraded: bool,
    sessions: u64,
    socket_mode: &'a str,
    max_frame_bytes: usize,
    metrics: MetricsSnapshot,
    active: RedactedActiveStatus,
    /// The session helper's content-free report, when the broker sends one.
    #[serde(skip_serializing_if = "Option::is_none")]
    desktop: Option<DesktopStatusPayload>,
    /// The apps and sites snoozed now, by settings identity.
    #[serde(skip_serializing_if = "<[SnoozeStatus]>::is_empty")]
    snoozes: &'a [SnoozeStatus],
}

#[derive(Debug, Serialize)]
struct Overview {
    schema: &'static str,
    broker: OverviewBroker,
    settings: SettingsV2,
    privacy: OverviewPrivacy,
    models: OverviewModels,
}

#[derive(Debug, Serialize)]
struct OverviewBroker {
    reachable: bool,
    provider: ProviderKind,
    paused: bool,
    authority_epoch: u64,
    settings_revision: u64,
    control_plane_degraded: bool,
    sessions: u64,
    socket_mode: String,
    max_frame_bytes: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    snoozes: Vec<SnoozeStatus>,
    /// The session helper's content-free report, when the broker sends one.
    #[serde(skip_serializing_if = "Option::is_none")]
    desktop: Option<DesktopStatusPayload>,
}

#[derive(Debug, Serialize)]
// These booleans are independent report facts in the versioned JSON contract.
#[allow(clippy::struct_excessive_bools)]
struct OverviewPrivacy {
    context: &'static str,
    max_before_chars: usize,
    max_after_chars: usize,
    clipboard: bool,
    screen: bool,
    network: bool,
    adaptive_writing_memory: &'static str,
    outcome_aggregates: &'static str,
    aggregate_semantics: &'static str,
    stored_metadata: &'static str,
    max_retention_days: Option<u16>,
    learning_available: bool,
}

#[derive(Debug, Serialize)]
struct OverviewModels {
    writing: OverviewModel,
}

#[derive(Debug, Serialize)]
struct OverviewModel {
    advice: ModelAdvice,
    configured: bool,
    installed: bool,
}

fn redact_health_status(status: &HealthStatusPayload) -> RedactedHealthStatus<'_> {
    RedactedHealthStatus {
        provider: status.provider,
        paused: status.paused,
        authority_epoch: status.authority_epoch,
        settings_revision: status.settings_revision,
        control_plane_degraded: status.control_plane_degraded,
        sessions: status.sessions,
        socket_mode: &status.socket_mode,
        max_frame_bytes: status.max_frame_bytes,
        metrics: status.metrics,
        active: RedactedActiveStatus {
            present: status.active.is_some(),
            has_suggestion: status
                .active
                .as_ref()
                .is_some_and(|active| active.suggestion_id.is_some()),
        },
        desktop: status.desktop,
        snoozes: &status.snoozes,
    }
}

// Keeping assembly in one place makes the strict overview contract auditable.
#[allow(clippy::too_many_lines)]
fn build_overview(
    health: HealthStatusPayload,
    status: SettingsStatusPayload,
) -> Result<Overview, CliError> {
    if !has_current_settings_schema(&status.document) {
        return Err(CliError::UnexpectedResponse);
    }
    let settings: SettingsV2 =
        serde_json::from_value(status.document).map_err(|_| CliError::UnexpectedResponse)?;
    settings
        .validate()
        .map_err(|_| CliError::UnexpectedResponse)?;
    if health.settings_revision != settings.revision {
        return Err(CliError::InconsistentOverview);
    }
    let max_retention_days = settings
        .subjects
        .iter()
        .filter_map(|subject| match subject.permissions.retention {
            RetentionPermission::None => None,
            RetentionPermission::Bounded { days } => Some(days),
        })
        .max();
    let persisted_aggregates = settings.subjects.iter().any(|subject| {
        subject.permissions.learn == PermissionDecision::Allow
            && matches!(
                subject.permissions.retention,
                RetentionPermission::Bounded { .. }
            )
    });
    let memory_only_aggregates = !persisted_aggregates
        && settings.subjects.iter().any(|subject| {
            subject.permissions.learn == PermissionDecision::Allow
                && matches!(subject.permissions.retention, RetentionPermission::None)
        });
    let writing = recommend_model(detect_hardware(), ModelUseCase::Writing);
    Ok(Overview {
        schema: "badi.overview.v2",
        broker: OverviewBroker {
            reachable: true,
            provider: health.provider,
            paused: health.paused,
            authority_epoch: health.authority_epoch,
            settings_revision: health.settings_revision,
            control_plane_degraded: health.control_plane_degraded,
            sessions: health.sessions,
            socket_mode: health.socket_mode,
            max_frame_bytes: health.max_frame_bytes,
            snoozes: health.snoozes,
            desktop: health.desktop,
        },
        settings,
        privacy: OverviewPrivacy {
            context: "focused_supported_field_only",
            max_before_chars: MAX_BEFORE_CHARS,
            max_after_chars: MAX_AFTER_CHARS,
            clipboard: false,
            screen: false,
            network: false,
            adaptive_writing_memory: "not_implemented",
            outcome_aggregates: if persisted_aggregates {
                "persisted"
            } else if memory_only_aggregates {
                "memory_only"
            } else {
                "disabled"
            },
            aggregate_semantics: "broker_emitted_and_commit_requested_not_delivery_confirmed",
            stored_metadata: "origin_provider_utc_day_counts",
            max_retention_days,
            learning_available: false,
        },
        models: OverviewModels {
            writing: OverviewModel {
                advice: writing,
                configured: health.provider == ProviderKind::LocalModel,
                installed: health.provider == ProviderKind::LocalModel,
            },
        },
    })
}

async fn request_coherent_overview(stream: &mut UnixStream) -> Result<Overview, CliError> {
    for _ in 0..OVERVIEW_SNAPSHOT_ATTEMPTS {
        let settings = request_settings(stream).await?;
        let health = request_health(stream).await?;
        match build_overview(health, settings) {
            Ok(overview) => return Ok(overview),
            Err(CliError::InconsistentOverview) => {}
            Err(error) => return Err(error),
        }
    }
    Err(CliError::InconsistentOverview)
}

async fn connect(path: &Path) -> Result<UnixStream, CliError> {
    verify_socket_metadata(path)?;
    let mut stream = UnixStream::connect(path).await?;
    verify_peer_uid(&stream)?;
    let mut hello = cli_global(
        MessageType::Hello,
        &HelloPayload {
            min_v: CURRENT_PROTOCOL_VERSION,
            max_v: CURRENT_PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Cli,
                name: "badictl".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: CLI_CAPABILITIES.to_vec(),
        },
    )?;
    let request_id = new_request_id();
    hello.id = Some(request_id.clone());
    tokio::time::timeout(HANDSHAKE_TIMEOUT, write_envelope(&mut stream, &hello))
        .await
        .map_err(|_| CliError::Handshake)??;
    let acknowledgment = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_envelope(&mut stream))
        .await
        .map_err(|_| CliError::Handshake)??
        .ok_or(CliError::ConnectionClosed)?;
    validate_handshake(&acknowledgment, &request_id)?;
    Ok(stream)
}

fn validate_handshake(
    acknowledgment: &WireEnvelope,
    request_id: &str,
) -> Result<HelloAckPayload, CliError> {
    if acknowledgment.v != CURRENT_PROTOCOL_VERSION
        || acknowledgment.validate_shape().is_err()
        || acknowledgment.message_type != MessageType::HelloAck
        || acknowledgment.id.as_deref() != Some(request_id)
    {
        return Err(CliError::Handshake);
    }
    let payload: HelloAckPayload = acknowledgment
        .decode_payload()
        .map_err(|_| CliError::Handshake)?;
    payload.validate().map_err(|_| CliError::Handshake)?;
    if payload.selected_v != CURRENT_PROTOCOL_VERSION
        || payload.enabled_capabilities.len() != CLI_CAPABILITIES.len()
        || !CLI_CAPABILITIES
            .iter()
            .all(|required| payload.enabled_capabilities.contains(required))
    {
        return Err(CliError::Handshake);
    }
    Ok(payload)
}

async fn request_health(stream: &mut UnixStream) -> Result<HealthStatusPayload, CliError> {
    let request = cli_global(MessageType::HealthRequest, &serde_json::json!({}))?;
    exchange(stream, request, validate_health_response).await
}

async fn request_settings(stream: &mut UnixStream) -> Result<SettingsStatusPayload, CliError> {
    let request = cli_global(MessageType::SettingsGet, &serde_json::json!({}))?;
    exchange(stream, request, validate_settings_response).await
}

/// The whole accepted control reply as JSON.
async fn request_control(
    stream: &mut UnixStream,
    request: WireEnvelope,
    action: ControlAction,
) -> Result<String, CliError> {
    exchange(stream, request, |response, request_id| {
        validate_control_response(response, request_id, action)?;
        Ok(serde_json::to_string(response)?)
    })
    .await
}

/// Sends `request` under a fresh correlation id and returns its one reply as
/// `validate` accepts it.
async fn exchange<T>(
    stream: &mut UnixStream,
    mut request: WireEnvelope,
    validate: impl FnOnce(&WireEnvelope, &str) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let request_id = new_request_id();
    request.id = Some(request_id.clone());
    write_envelope(stream, &request).await?;
    let response = tokio::time::timeout(RESPONSE_TIMEOUT, read_envelope(stream))
        .await
        .map_err(|_| CliError::ResponseTimeout)??
        .ok_or(CliError::ConnectionClosed)?;
    validate(&response, &request_id)
}

fn validate_settings_response(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<SettingsStatusPayload, CliError> {
    let payload = decode_reply(
        response,
        request_id,
        MessageType::SettingsStatus,
        SettingsStatusPayload::validate,
    )?;
    if !has_current_settings_schema(&payload.document) {
        return Err(CliError::UnexpectedResponse);
    }
    let settings: SettingsV2 = serde_json::from_value(payload.document.clone())
        .map_err(|_| CliError::UnexpectedResponse)?;
    settings
        .validate()
        .map_err(|_| CliError::UnexpectedResponse)?;
    Ok(payload)
}

fn has_current_settings_schema(document: &Value) -> bool {
    document.get("schema").and_then(Value::as_str) == Some(SETTINGS_SCHEMA)
}

fn addressed_control(
    action: ControlAction,
    active: ActiveLocator,
) -> Result<WireEnvelope, CliError> {
    let needs_suggestion = matches!(
        action,
        ControlAction::AcceptWord | ControlAction::AcceptAll | ControlAction::Dismiss
    );
    if needs_suggestion && active.suggestion_id.is_none() {
        return Err(CliError::NoSuggestion);
    }
    Ok(cli_session(
        MessageType::ControlRequest,
        Coordinates {
            session_id: active.session_id,
            focus_epoch: active.focus_epoch,
            revision: active.revision,
        },
        &SessionControlRequestPayload {
            action,
            fingerprint: active.fingerprint,
            suggestion_id: active.suggestion_id,
        },
    )?)
}

fn validate_correlated_response(
    response: &WireEnvelope,
    request_id: &str,
    expected_type: MessageType,
) -> Result<(), CliError> {
    if response.v != CURRENT_PROTOCOL_VERSION
        || response.validate_shape().is_err()
        || response.id.as_deref() != Some(request_id)
    {
        return Err(CliError::UnexpectedResponse);
    }
    if response.message_type == MessageType::Error {
        let payload: ErrorPayload = response
            .decode_payload()
            .map_err(|_| CliError::UnexpectedResponse)?;
        payload
            .validate()
            .map_err(|_| CliError::UnexpectedResponse)?;
        let code = serde_json::to_value(payload.code)?
            .as_str()
            .ok_or(CliError::UnexpectedResponse)?
            .to_owned();
        if let (Some(settings_revision), Some(degraded)) =
            (payload.settings_revision, payload.control_plane_degraded)
        {
            return Err(CliError::SettingsRejected {
                code,
                committed: payload
                    .committed
                    .map_or("unknown", |value| if value { "true" } else { "false" }),
                settings_revision,
                degraded,
            });
        }
        return Err(CliError::Rejected(code));
    }
    if response.message_type != expected_type {
        return Err(CliError::UnexpectedResponse);
    }
    Ok(())
}

fn validate_control_response(
    response: &WireEnvelope,
    request_id: &str,
    expected_action: ControlAction,
) -> Result<ControlResultPayload, CliError> {
    validate_correlated_response(response, request_id, MessageType::ControlResult)?;
    let payload: ControlResultPayload = response
        .decode_payload()
        .map_err(|_| CliError::UnexpectedResponse)?;
    if payload.action != expected_action {
        return Err(CliError::UnexpectedResponse);
    }
    // A snooze needs one focused field Badi follows; say why there was none.
    if expected_action == ControlAction::Snooze
        && !payload.accepted
        && matches!(
            payload.reason,
            ReasonCode::NoContext | ReasonCode::AmbiguousSession | ReasonCode::Paused
        )
        && payload.snooze.is_none()
    {
        let reason = serde_json::to_value(payload.reason)?;
        return Err(CliError::Rejected(
            reason.as_str().unwrap_or("unexpected").to_owned(),
        ));
    }
    if !payload.accepted
        || payload.reason != ReasonCode::Accepted
        || payload.snooze.is_some() != (expected_action == ControlAction::Snooze)
        || match expected_action {
            ControlAction::Pause => !payload.paused,
            ControlAction::PauseToggle | ControlAction::Resume | ControlAction::SnoozeEnd => false,
            ControlAction::Snooze
            | ControlAction::Request
            | ControlAction::AcceptWord
            | ControlAction::AcceptAll
            | ControlAction::Dismiss => payload.paused,
        }
    {
        return Err(CliError::UnexpectedResponse);
    }
    Ok(payload)
}

fn validate_health_response(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<HealthStatusPayload, CliError> {
    decode_reply(
        response,
        request_id,
        MessageType::HealthStatus,
        HealthStatusPayload::validate,
    )
}

fn validate_memory_response(
    response: &WireEnvelope,
    request_id: &str,
) -> Result<MemoryStatusPayload, CliError> {
    decode_reply(
        response,
        request_id,
        MessageType::MemoryStatus,
        MemoryStatusPayload::validate,
    )
}

/// Decodes a correlated `reply_type` reply whose payload passes `validate`.
fn decode_reply<T: DeserializeOwned>(
    response: &WireEnvelope,
    request_id: &str,
    reply_type: MessageType,
    validate: fn(&T) -> Result<(), ProtocolError>,
) -> Result<T, CliError> {
    validate_correlated_response(response, request_id, reply_type)?;
    let payload: T = response
        .decode_payload()
        .map_err(|_| CliError::UnexpectedResponse)?;
    validate(&payload).map_err(|_| CliError::UnexpectedResponse)?;
    Ok(payload)
}

fn new_request_id() -> String {
    format!("ctl:{}", uuid::Uuid::new_v4())
}

fn cli_global<P: Serialize>(
    message_type: MessageType,
    payload: &P,
) -> Result<WireEnvelope, ProtocolError> {
    WireEnvelope::global(message_type, 0, payload)
}

fn cli_session<P: Serialize>(
    message_type: MessageType,
    coordinates: Coordinates,
    payload: &P,
) -> Result<WireEnvelope, ProtocolError> {
    WireEnvelope::session(message_type, coordinates, 0, payload)
}

const CLI_USAGE: &str = "Usage: badictl [--socket ABSOLUTE] COMMAND\n\
Local commands:\n  hardware [--json]       Inspect content-free hardware capabilities\n  models [USE] [--json]   Recommend pinned local models; USE is writing or code\n\
Broker commands:\n  status [--json]  Show content-free broker status as JSON\n  request          Request a suggestion for the sole active session\n  accept-word      Accept the authorized first word-part\n  accept-all       Accept the authorized full suggestion\n  dismiss          Dismiss the current suggestion\n  pause [MODE]     MODE is on, off, or toggle (default)\n\
  snooze [MINUTES] Withhold suggestions in the focused field's app or site\n                    for MINUTES (1-1440, default 10); runtime only\n\
  snooze end       End every snooze now\n\
  overview [--json]  Show broker, policy, privacy, and model readiness\n  settings show [--json]\n                    Show the strict badi.settings.v2 document\n  settings replace --if-revision N --json DOCUMENT|-\n                    Replace settings with compare-and-swap protection; - reads\n                    a bounded document from stdin, keeping it out of argv\n  memory clear      Clear local text-free origin/day interaction aggregates\n\
  context status    Show content-free context state and learned-word counts\n\
  context forget    Delete every learned word (the keyring key stays)\n\
  stats status      Show content-free counts per day and per app or site\n\
  stats forget      Delete every usage count\n\
  style status      Show content-free style-learning state\n\
  style forget      Delete every style sample and forget the key in memory\n\
  style export      Print decrypted style samples as JSON (explicit; for the Lab)\n\
  style key -       Deliver the keyring key, read as \"KEY_ID KEY\" hex from stdin\n\
  probe [--language TAG] [--after TEXT] [--replace] [--explicit] [--] TEXT|-\n                    Run TEXT (before the caret) through the live provider and\n                    display checks; print JSON. The broker opens no session or\n                    commit and stores nothing. Arguments stay in shell history\n                    and the process list; - reads TEXT from stdin instead.\n                    TAG defaults to en; --replace allows spelling replacement;\n                    --explicit uses the Tab-request budget (1200 ms, not 550 ms)\n\
Options:\n  --socket ABSOLUTE  Override $XDG_RUNTIME_DIR/badi/broker.sock\n  -h, --help         Show this help\n  --version          Print the version and embedded source commit\n";

fn parse_arguments<I>(arguments: I) -> Result<ParsedCommand, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut arguments = arguments.into_iter().peekable();
    if arguments
        .peek()
        .is_some_and(|argument| argument == "--help" || argument == "-h")
    {
        let _ = arguments.next();
        return if arguments.next().is_none() {
            Ok(ParsedCommand::Help)
        } else {
            Err(CliError::Arguments)
        };
    }
    if arguments
        .peek()
        .is_some_and(|argument| argument == "--version")
    {
        let _ = arguments.next();
        return if arguments.next().is_none() {
            Ok(ParsedCommand::Version)
        } else {
            Err(CliError::Arguments)
        };
    }

    let explicit_socket = if arguments
        .peek()
        .is_some_and(|argument| argument == "--socket")
    {
        let _ = arguments.next();
        let value = arguments.next().ok_or(CliError::Arguments)?;
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(CliError::Arguments);
        }
        Some(path)
    } else {
        None
    };

    let command_name = arguments.next();
    let rest: Vec<String> = arguments.collect();
    if command_name.as_deref() == Some("hardware") {
        if explicit_socket.is_some() || !json_only(&rest) {
            return Err(CliError::Arguments);
        }
        return Ok(ParsedCommand::Local(LocalCommand::Hardware));
    }
    if command_name.as_deref() == Some("models") {
        if explicit_socket.is_some() {
            return Err(CliError::Arguments);
        }
        let use_case = parse_model_use_case(&rest)?;
        return Ok(ParsedCommand::Local(LocalCommand::Models(use_case)));
    }

    let command = match command_name.as_deref() {
        Some("--help" | "-h") => {
            if !rest.is_empty() {
                return Err(CliError::Arguments);
            }
            return Ok(ParsedCommand::Help);
        }
        Some("status") => {
            if !json_only(&rest) {
                return Err(CliError::Arguments);
            }
            Command::Status
        }
        Some("overview") => {
            if !json_only(&rest) {
                return Err(CliError::Arguments);
            }
            Command::Overview
        }
        Some("settings") => parse_settings_command(&rest)?,
        Some("probe") => parse_probe_command(rest)?,
        Some("memory") if rest == ["clear"] => Command::MemoryClear,
        Some("style") => parse_style_command(&rest)?,
        Some("stats") => Command::Stats(status_or_forget(
            &rest,
            StatsCommand::Status,
            StatsCommand::Forget,
        )?),
        Some("context") => Command::Context(status_or_forget(
            &rest,
            ContextCommand::Status,
            ContextCommand::Forget,
        )?),
        Some("request") if rest.is_empty() => Command::Session(ControlAction::Request),
        Some("accept-word") if rest.is_empty() => Command::Session(ControlAction::AcceptWord),
        Some("accept-all") if rest.is_empty() => Command::Session(ControlAction::AcceptAll),
        Some("dismiss") if rest.is_empty() => Command::Session(ControlAction::Dismiss),
        Some("pause") => global(parse_pause(&rest)?),
        Some("snooze") => parse_snooze_command(&rest)?,
        _ => return Err(CliError::Arguments),
    };
    let socket_path = explicit_socket.map_or_else(default_socket_path, Ok)?;
    Ok(ParsedCommand::Remote {
        socket_path,
        command,
    })
}

const fn global(action: ControlAction) -> Command {
    Command::Global(GlobalControlRequestPayload {
        action,
        minutes: None,
    })
}

/// `snooze [MINUTES]` (10 by default) or `snooze end`.
fn parse_snooze_command(arguments: &[String]) -> Result<Command, CliError> {
    let minutes = match arguments {
        [] => DEFAULT_SNOOZE_MINUTES,
        [value] if value == "end" => return Ok(global(ControlAction::SnoozeEnd)),
        [value] => value
            .parse::<u16>()
            .ok()
            .filter(|minutes| (1..=MAX_SNOOZE_MINUTES).contains(minutes))
            .ok_or(CliError::Arguments)?,
        _ => return Err(CliError::Arguments),
    };
    Ok(Command::Global(GlobalControlRequestPayload {
        action: ControlAction::Snooze,
        minutes: Some(minutes),
    }))
}

fn parse_pause(arguments: &[String]) -> Result<ControlAction, CliError> {
    match arguments {
        [] => Ok(ControlAction::PauseToggle),
        [value] if value == "toggle" => Ok(ControlAction::PauseToggle),
        [value] if value == "on" => Ok(ControlAction::Pause),
        [value] if value == "off" => Ok(ControlAction::Resume),
        _ => Err(CliError::Arguments),
    }
}

/// `status` or `forget`: the two verbs of a content-free local store.
fn status_or_forget<T>(arguments: &[String], status: T, forget: T) -> Result<T, CliError> {
    match arguments {
        [value] if value == "status" => Ok(status),
        [value] if value == "forget" => Ok(forget),
        _ => Err(CliError::Arguments),
    }
}

fn parse_style_command(arguments: &[String]) -> Result<Command, CliError> {
    Ok(Command::Style(match arguments {
        [value] if value == "status" => StyleCommand::Status,
        [value] if value == "forget" => StyleCommand::Forget,
        [value] if value == "export" => StyleCommand::Export,
        [value, from] if value == "key" && from == "-" => StyleCommand::Key(None),
        _ => return Err(CliError::Arguments),
    }))
}

fn parse_probe_command(arguments: Vec<String>) -> Result<Command, CliError> {
    let mut language = Some("en".to_owned());
    let mut after = String::new();
    let mut allow_replacement = false;
    let mut explicit = false;
    let mut arguments = arguments.into_iter();
    let (before, text_from_stdin) = loop {
        let argument = arguments.next().ok_or(CliError::Arguments)?;
        match argument.as_str() {
            "--language" => language = Some(arguments.next().ok_or(CliError::Arguments)?),
            "--after" => after = arguments.next().ok_or(CliError::Arguments)?,
            "--replace" => allow_replacement = true,
            "--explicit" => explicit = true,
            "--" => break (arguments.next().ok_or(CliError::Arguments)?, false),
            "-" => break (String::new(), true),
            value if value.starts_with("--") => return Err(CliError::Arguments),
            _ => break (argument, false),
        }
    };
    if arguments.next().is_some() {
        return Err(CliError::Arguments);
    }
    let payload = ProbeRequestPayload {
        before,
        after,
        language,
        allow_replacement,
        explicit,
    };
    payload.validate().map_err(|_| CliError::Arguments)?;
    Ok(Command::Probe {
        payload,
        text_from_stdin,
    })
}

fn json_only(arguments: &[String]) -> bool {
    arguments.is_empty() || arguments.len() == 1 && arguments[0] == "--json"
}

fn parse_model_use_case(arguments: &[String]) -> Result<ModelUseCase, CliError> {
    match arguments {
        [] => Ok(ModelUseCase::Writing),
        [value] if value == "writing" || value == "--json" => Ok(ModelUseCase::Writing),
        [value] if value == "code" => Ok(ModelUseCase::Code),
        [value, format] if value == "writing" && format == "--json" => Ok(ModelUseCase::Writing),
        [value, format] if value == "code" && format == "--json" => Ok(ModelUseCase::Code),
        _ => Err(CliError::Arguments),
    }
}

fn parse_settings_command(arguments: &[String]) -> Result<Command, CliError> {
    match arguments {
        [command] if command == "show" => Ok(Command::SettingsShow),
        [command, format] if command == "show" && format == "--json" => Ok(Command::SettingsShow),
        [command, revision_flag, revision, json_flag, document]
            if command == "replace"
                && revision_flag == "--if-revision"
                && json_flag == "--json" =>
        {
            let expected_revision = revision.parse::<u64>().map_err(|_| CliError::Arguments)?;
            if expected_revision >= MAX_SAFE_COUNTER {
                return Err(CliError::Arguments);
            }
            if document == "-" {
                Ok(Command::SettingsReplaceStdin { expected_revision })
            } else {
                settings_replacement(expected_revision, document)
            }
        }
        _ => Err(CliError::Arguments),
    }
}

fn settings_replacement(expected_revision: u64, json: &str) -> Result<Command, CliError> {
    let document: Value = serde_json::from_str(json).map_err(|_| CliError::Arguments)?;
    if !has_current_settings_schema(&document) {
        return Err(CliError::Arguments);
    }
    let settings: SettingsV2 =
        serde_json::from_value(document.clone()).map_err(|_| CliError::Arguments)?;
    settings.validate().map_err(|_| CliError::Arguments)?;
    let next_revision = expected_revision
        .checked_add(1)
        .filter(|revision| *revision <= MAX_SAFE_COUNTER)
        .ok_or(CliError::Arguments)?;
    if settings.revision != next_revision {
        return Err(CliError::Arguments);
    }
    Ok(Command::SettingsReplace {
        expected_revision,
        document,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Global(GlobalControlRequestPayload),
    MemoryClear,
    Overview,
    Probe {
        payload: ProbeRequestPayload,
        /// The positional TEXT was `-`; `payload.before` is filled from stdin.
        text_from_stdin: bool,
    },
    Session(ControlAction),
    SettingsReplace {
        expected_revision: u64,
        document: Value,
    },
    SettingsReplaceStdin {
        expected_revision: u64,
    },
    SettingsShow,
    Status,
    Style(StyleCommand),
    Context(ContextCommand),
    Stats(StatsCommand),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatsCommand {
    Status,
    Forget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContextCommand {
    Status,
    Forget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StyleCommand {
    Status,
    Forget,
    Export,
    /// `None` until stdin supplies the key.
    Key(Option<StyleKeyPayload>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalCommand {
    Hardware,
    Models(ModelUseCase),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ParsedCommand {
    Remote {
        socket_path: PathBuf,
        command: Command,
    },
    Local(LocalCommand),
    Help,
    Version,
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("arguments")]
    Arguments,
    #[error("connection_closed")]
    ConnectionClosed,
    #[error("frame")]
    Frame(#[from] badi_broker::ipc::FrameError),
    #[error("handshake")]
    Handshake,
    #[error("io")]
    Io(#[from] std::io::Error),
    #[error("inconsistent_overview")]
    InconsistentOverview,
    #[error("no_active_session")]
    NoActiveSession,
    #[error("no_suggestion")]
    NoSuggestion,
    #[error("protocol")]
    Protocol(#[from] ProtocolError),
    #[error("rejected:{0}")]
    Rejected(String),
    #[error("response_timeout")]
    ResponseTimeout,
    #[error("serde")]
    Serde(#[from] serde_json::Error),
    #[error(
        "settings_rejected:code={code}:committed={committed}:settings_revision={settings_revision}:degraded={degraded}"
    )]
    SettingsRejected {
        code: String,
        committed: &'static str,
        settings_revision: u64,
        degraded: bool,
    },
    #[error("unexpected_response")]
    UnexpectedResponse,
}

#[cfg(test)]
mod tests;
