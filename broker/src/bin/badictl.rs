use std::path::{Path, PathBuf};
use std::time::Duration;

use badi_broker::ipc::{
    default_socket_path, read_envelope, verify_peer_uid, verify_socket_metadata, write_envelope,
};
use badi_broker::metrics::MetricsSnapshot;
use badi_broker::model_selection::{ModelUseCase, detect_hardware, recommend_model};
use badi_broker::protocol::{
    ActiveLocator, AdapterDescriptor, AdapterKind, Capability, ControlAction, ControlResultPayload,
    ErrorPayload, GlobalControlRequestPayload, HealthStatusPayload, HelloAckPayload, HelloPayload,
    MessageType, PROTOCOL_VERSION, ProviderKind, ReasonCode, SessionControlRequestPayload,
    WireEnvelope,
};
use serde::Serialize;
use tokio::net::UnixStream;

const CLI_CAPABILITIES: [Capability; 2] = [Capability::Control, Capability::Health];
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::main]
async fn main() {
    let result = run().await;
    if let Err(error) = result {
        eprintln!("error_code={error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), CliError> {
    let (socket_path, command) = match parse_arguments(std::env::args().skip(1))? {
        ParsedCommand::Help => {
            print!("{CLI_USAGE}");
            return Ok(());
        }
        ParsedCommand::Local(LocalCommand::Hardware) => {
            println!("{}", serde_json::to_string_pretty(&detect_hardware())?);
            return Ok(());
        }
        ParsedCommand::Local(LocalCommand::Models(use_case)) => {
            let advice = recommend_model(detect_hardware(), use_case);
            println!("{}", serde_json::to_string_pretty(&advice)?);
            return Ok(());
        }
        ParsedCommand::Remote {
            socket_path,
            command,
        } => (socket_path, command),
    };
    let mut stream = connect(&socket_path).await?;
    match command {
        Command::Status => {
            let status = request_health(&mut stream).await?;
            println!("{}", serde_json::to_string(&redact_health_status(&status))?);
        }
        Command::Global(action) => {
            let mut request = WireEnvelope::global(
                MessageType::ControlRequest,
                0,
                &GlobalControlRequestPayload { action },
            )?;
            let request_id = new_request_id();
            request.id = Some(request_id.clone());
            write_envelope(&mut stream, &request).await?;
            print_control_response(&mut stream, &request_id, action).await?;
        }
        Command::Session(action) => {
            let status = request_health(&mut stream).await?;
            let active = status.active.ok_or(CliError::NoActiveSession)?;
            let request = addressed_control(action, active)?;
            let request_id = request.id.clone().ok_or(CliError::UnexpectedResponse)?;
            write_envelope(&mut stream, &request).await?;
            print_control_response(&mut stream, &request_id, action).await?;
        }
    }
    Ok(())
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
    sessions: u64,
    socket_mode: &'a str,
    max_frame_bytes: usize,
    metrics: MetricsSnapshot,
    active: RedactedActiveStatus,
}

fn redact_health_status(status: &HealthStatusPayload) -> RedactedHealthStatus<'_> {
    RedactedHealthStatus {
        provider: status.provider,
        paused: status.paused,
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
    }
}

async fn connect(path: &Path) -> Result<UnixStream, CliError> {
    verify_socket_metadata(path)?;
    let mut stream = UnixStream::connect(path).await?;
    verify_peer_uid(&stream)?;
    let mut hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: PROTOCOL_VERSION,
            max_v: PROTOCOL_VERSION,
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
    if acknowledgment.validate_shape().is_err()
        || acknowledgment.message_type != MessageType::HelloAck
        || acknowledgment.id.as_deref() != Some(request_id)
    {
        return Err(CliError::Handshake);
    }
    let payload: HelloAckPayload = acknowledgment
        .decode_payload()
        .map_err(|_| CliError::Handshake)?;
    payload.validate().map_err(|_| CliError::Handshake)?;
    if payload.enabled_capabilities.len() != CLI_CAPABILITIES.len()
        || !CLI_CAPABILITIES
            .iter()
            .all(|required| payload.enabled_capabilities.contains(required))
    {
        return Err(CliError::Handshake);
    }
    Ok(payload)
}

async fn request_health(stream: &mut UnixStream) -> Result<HealthStatusPayload, CliError> {
    let mut request = WireEnvelope::global(MessageType::HealthRequest, 0, &serde_json::json!({}))?;
    let request_id = new_request_id();
    request.id = Some(request_id.clone());
    write_envelope(stream, &request).await?;
    let response = read_correlated_response(stream, &request_id, MessageType::HealthStatus).await?;
    validate_health_response(&response, &request_id)
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
    let mut envelope = WireEnvelope::session(
        MessageType::ControlRequest,
        badi_broker::protocol::Coordinates {
            session_id: active.session_id,
            focus_epoch: active.focus_epoch,
            revision: active.revision,
        },
        0,
        &SessionControlRequestPayload {
            action,
            fingerprint: active.fingerprint,
            suggestion_id: active.suggestion_id,
        },
    )?;
    envelope.id = Some(new_request_id());
    Ok(envelope)
}

async fn read_correlated_response(
    stream: &mut UnixStream,
    request_id: &str,
    expected_type: MessageType,
) -> Result<WireEnvelope, CliError> {
    let response = tokio::time::timeout(RESPONSE_TIMEOUT, read_envelope(stream))
        .await
        .map_err(|_| CliError::ResponseTimeout)??
        .ok_or(CliError::ConnectionClosed)?;
    validate_correlated_response(&response, request_id, expected_type)?;
    Ok(response)
}

fn validate_correlated_response(
    response: &WireEnvelope,
    request_id: &str,
    expected_type: MessageType,
) -> Result<(), CliError> {
    if response.validate_shape().is_err() || response.id.as_deref() != Some(request_id) {
        return Err(CliError::UnexpectedResponse);
    }
    if response.message_type == MessageType::Error {
        let _: ErrorPayload = response
            .decode_payload()
            .map_err(|_| CliError::UnexpectedResponse)?;
        return Err(CliError::Rejected);
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
    if payload.action != expected_action
        || !payload.accepted
        || payload.reason != ReasonCode::Accepted
        || match expected_action {
            ControlAction::Pause => !payload.paused,
            ControlAction::PauseToggle => false,
            ControlAction::Resume
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
    validate_correlated_response(response, request_id, MessageType::HealthStatus)?;
    let payload: HealthStatusPayload = response
        .decode_payload()
        .map_err(|_| CliError::UnexpectedResponse)?;
    if payload.socket_mode != "0600"
        || payload.max_frame_bytes != badi_broker::protocol::MAX_FRAME_BYTES
    {
        return Err(CliError::UnexpectedResponse);
    }
    Ok(payload)
}

async fn print_control_response(
    stream: &mut UnixStream,
    request_id: &str,
    expected_action: ControlAction,
) -> Result<(), CliError> {
    let response = tokio::time::timeout(RESPONSE_TIMEOUT, read_envelope(stream))
        .await
        .map_err(|_| CliError::ResponseTimeout)??
        .ok_or(CliError::ConnectionClosed)?;
    let _ = validate_control_response(&response, request_id, expected_action)?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

fn new_request_id() -> String {
    format!("ctl:{}", uuid::Uuid::new_v4())
}

const CLI_USAGE: &str = "Usage: badictl [--socket ABSOLUTE] COMMAND\n\
Local commands:\n  hardware [--json]       Inspect content-free hardware capabilities\n  models [USE] [--json]   Recommend pinned local models; USE is writing or code\n\
Broker commands:\n  status [--json]  Show content-free broker status as JSON\n  request          Request a suggestion for the sole active session\n  accept-word      Accept the authorized first word-part\n  accept-all       Accept the authorized full suggestion\n  dismiss          Dismiss the current suggestion\n  pause [MODE]     MODE is on, off, or toggle (default)\n\
Options:\n  --socket ABSOLUTE  Override $XDG_RUNTIME_DIR/badi/broker.sock\n  -h, --help         Show this help\n";

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
        Some("request") if rest.is_empty() => Command::Session(ControlAction::Request),
        Some("accept-word") if rest.is_empty() => Command::Session(ControlAction::AcceptWord),
        Some("accept-all") if rest.is_empty() => Command::Session(ControlAction::AcceptAll),
        Some("dismiss") if rest.is_empty() => Command::Session(ControlAction::Dismiss),
        Some("pause") => match rest.as_slice() {
            [] => Command::Global(ControlAction::PauseToggle),
            [value] if value == "toggle" => Command::Global(ControlAction::PauseToggle),
            [value] if value == "on" => Command::Global(ControlAction::Pause),
            [value] if value == "off" => Command::Global(ControlAction::Resume),
            _ => return Err(CliError::Arguments),
        },
        _ => return Err(CliError::Arguments),
    };
    let socket_path = explicit_socket.map_or_else(default_socket_path, Ok)?;
    Ok(ParsedCommand::Remote {
        socket_path,
        command,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Global(ControlAction),
    Session(ControlAction),
    Status,
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
    #[error("no_active_session")]
    NoActiveSession,
    #[error("no_suggestion")]
    NoSuggestion,
    #[error("protocol")]
    Protocol(#[from] badi_broker::protocol::ProtocolError),
    #[error("rejected")]
    Rejected,
    #[error("response_timeout")]
    ResponseTimeout,
    #[error("serde")]
    Serde(#[from] serde_json::Error),
    #[error("unexpected_response")]
    UnexpectedResponse,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        CliError, Command, LocalCommand, ParsedCommand, parse_arguments, redact_health_status,
        validate_control_response, validate_correlated_response, validate_handshake,
        validate_health_response,
    };
    use badi_broker::metrics::MetricsSnapshot;
    use badi_broker::model_selection::ModelUseCase;
    use badi_broker::protocol::{
        ActiveLocator, Capability, ControlAction, ControlResultPayload, HealthStatusPayload,
        HelloAckPayload, MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_FRAME_BYTES, MAX_SUGGESTION_CHARS,
        MAX_SUGGESTION_WORDS, MessageType, PROTOCOL_VERSION, ProviderKind, ReasonCode, SessionId,
        WireEnvelope,
    };
    use serde_json::json;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn hello_acknowledgment(request_id: &str) -> WireEnvelope {
        let mut acknowledgment = WireEnvelope::global(
            MessageType::HelloAck,
            0,
            &HelloAckPayload {
                selected_v: PROTOCOL_VERSION,
                connection_id: "c:test-connection".to_owned(),
                enabled_capabilities: vec![Capability::Control, Capability::Health],
                max_frame_bytes: MAX_FRAME_BYTES,
                max_before_chars: MAX_BEFORE_CHARS,
                max_after_chars: MAX_AFTER_CHARS,
                max_suggestion_chars: MAX_SUGGESTION_CHARS,
                max_suggestion_words: MAX_SUGGESTION_WORDS,
                paused: true,
            },
        )
        .expect("hello acknowledgment");
        acknowledgment.id = Some(request_id.to_owned());
        acknowledgment
    }

    #[test]
    fn strict_handshake_accepts_correlated_complete_acknowledgment() {
        let payload = validate_handshake(&hello_acknowledgment("ctl:request"), "ctl:request")
            .expect("strict acknowledgment");
        assert!(payload.paused);
        assert_eq!(
            payload.enabled_capabilities,
            vec![Capability::Control, Capability::Health]
        );
    }

    #[test]
    fn strict_handshake_rejects_wrong_id_limits_capabilities_and_paused_shape() {
        let acknowledgment = hello_acknowledgment("ctl:other");
        assert!(matches!(
            validate_handshake(&acknowledgment, "ctl:request"),
            Err(CliError::Handshake)
        ));

        let mut wrong_limit = hello_acknowledgment("ctl:request");
        wrong_limit.payload["max_frame_bytes"] = json!(MAX_FRAME_BYTES - 1);
        assert!(matches!(
            validate_handshake(&wrong_limit, "ctl:request"),
            Err(CliError::Handshake)
        ));

        let mut wrong_version = hello_acknowledgment("ctl:request");
        wrong_version.payload["selected_v"] = json!(PROTOCOL_VERSION + 1);
        assert!(matches!(
            validate_handshake(&wrong_version, "ctl:request"),
            Err(CliError::Handshake)
        ));

        let mut missing_capability = hello_acknowledgment("ctl:request");
        missing_capability.payload["enabled_capabilities"] = json!(["control"]);
        assert!(matches!(
            validate_handshake(&missing_capability, "ctl:request"),
            Err(CliError::Handshake)
        ));

        let mut invalid_paused = hello_acknowledgment("ctl:request");
        invalid_paused.payload["paused"] = json!("false");
        assert!(matches!(
            validate_handshake(&invalid_paused, "ctl:request"),
            Err(CliError::Handshake)
        ));
    }

    fn control_response(request_id: &str, action: ControlAction) -> WireEnvelope {
        let mut response = WireEnvelope::global(
            MessageType::ControlResult,
            1,
            &ControlResultPayload {
                action,
                accepted: true,
                reason: ReasonCode::Accepted,
                paused: action == ControlAction::Pause,
            },
        )
        .expect("control response");
        response.id = Some(request_id.to_owned());
        response
    }

    #[test]
    fn post_hello_responses_require_matching_id_type_and_control_payload() {
        let response = control_response("ctl:request", ControlAction::Pause);
        assert!(validate_control_response(&response, "ctl:request", ControlAction::Pause).is_ok());
        assert!(matches!(
            validate_correlated_response(&response, "ctl:other", MessageType::ControlResult),
            Err(CliError::UnexpectedResponse)
        ));
        assert!(matches!(
            validate_correlated_response(&response, "ctl:request", MessageType::HealthStatus),
            Err(CliError::UnexpectedResponse)
        ));
        assert!(matches!(
            validate_control_response(&response, "ctl:request", ControlAction::Resume),
            Err(CliError::UnexpectedResponse)
        ));

        let mut contradictory_pause = control_response("ctl:request", ControlAction::Pause);
        contradictory_pause.payload["paused"] = json!(false);
        assert!(matches!(
            validate_control_response(&contradictory_pause, "ctl:request", ControlAction::Pause),
            Err(CliError::UnexpectedResponse)
        ));
        let mut contradictory_resume = control_response("ctl:request", ControlAction::Resume);
        contradictory_resume.payload["paused"] = json!(true);
        assert!(matches!(
            validate_control_response(&contradictory_resume, "ctl:request", ControlAction::Resume),
            Err(CliError::UnexpectedResponse)
        ));

        let mut rejected = control_response("ctl:request", ControlAction::Pause);
        rejected.payload["accepted"] = json!(false);
        assert!(matches!(
            validate_control_response(&rejected, "ctl:request", ControlAction::Pause),
            Err(CliError::UnexpectedResponse)
        ));
        let mut wrong_reason = control_response("ctl:request", ControlAction::Pause);
        wrong_reason.payload["reason"] = json!("provider_error");
        assert!(matches!(
            validate_control_response(&wrong_reason, "ctl:request", ControlAction::Pause),
            Err(CliError::UnexpectedResponse)
        ));
    }

    #[test]
    fn health_response_requires_correlated_type_and_fixed_transport_limits() {
        let mut response = WireEnvelope::global(
            MessageType::HealthStatus,
            1,
            &HealthStatusPayload {
                provider: ProviderKind::PhraseV1,
                paused: false,
                sessions: 0,
                socket_mode: "0600".to_owned(),
                max_frame_bytes: MAX_FRAME_BYTES,
                metrics: MetricsSnapshot::default(),
                active: None,
            },
        )
        .expect("health response");
        response.id = Some("ctl:health".to_owned());
        assert!(validate_health_response(&response, "ctl:health").is_ok());

        response.payload["max_frame_bytes"] = json!(MAX_FRAME_BYTES - 1);
        assert!(matches!(
            validate_health_response(&response, "ctl:health"),
            Err(CliError::UnexpectedResponse)
        ));
    }

    #[test]
    fn parses_help_without_requiring_a_runtime_directory() {
        assert_eq!(
            parse_arguments(arguments(&["--help"])).expect("help"),
            ParsedCommand::Help
        );
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "-h"]))
                .expect("help after socket"),
            ParsedCommand::Help
        );
    }

    #[test]
    fn parses_commands_with_absolute_socket() {
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "accept-word",]))
                .expect("accept word"),
            ParsedCommand::Remote {
                socket_path: PathBuf::from("/tmp/broker.sock"),
                command: Command::Session(ControlAction::AcceptWord),
            }
        );
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "pause", "off"]))
                .expect("pause off"),
            ParsedCommand::Remote {
                socket_path: PathBuf::from("/tmp/broker.sock"),
                command: Command::Global(ControlAction::Resume),
            }
        );
    }

    #[test]
    fn parses_local_hardware_and_model_commands_without_a_socket() {
        assert_eq!(
            parse_arguments(arguments(&["hardware"])).expect("hardware"),
            ParsedCommand::Local(LocalCommand::Hardware)
        );
        assert_eq!(
            parse_arguments(arguments(&["models", "code", "--json"])).expect("code models"),
            ParsedCommand::Local(LocalCommand::Models(ModelUseCase::Code))
        );
        assert_eq!(
            parse_arguments(arguments(&["models"])).expect("writing models"),
            ParsedCommand::Local(LocalCommand::Models(ModelUseCase::Writing))
        );
    }

    #[test]
    fn rejects_relative_socket_and_extra_arguments() {
        assert!(matches!(
            parse_arguments(arguments(&["--socket", "broker.sock", "status"])),
            Err(CliError::Arguments)
        ));
        assert!(matches!(
            parse_arguments(arguments(&["--help", "extra"])),
            Err(CliError::Arguments)
        ));
        assert!(matches!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "hardware"])),
            Err(CliError::Arguments)
        ));
    }

    #[test]
    fn status_json_redacts_all_active_locator_tokens() {
        let session_id = SessionId::new();
        let status = HealthStatusPayload {
            provider: ProviderKind::PhraseV1,
            paused: false,
            sessions: 1,
            socket_mode: "0600".to_owned(),
            max_frame_bytes: 65_536,
            metrics: MetricsSnapshot::default(),
            active: Some(ActiveLocator {
                session_id,
                focus_epoch: 7,
                revision: 11,
                fingerprint: "fingerprint_000000000001".to_owned(),
                suggestion_id: Some("s:secret".to_owned()),
            }),
        };

        let value = serde_json::to_value(redact_health_status(&status)).expect("redacted JSON");
        assert_eq!(
            value["active"],
            json!({ "present": true, "has_suggestion": true })
        );
        let encoded = serde_json::to_string(&value).expect("encoded redacted JSON");
        for secret in [
            &session_id.to_string(),
            "fingerprint_000000000001",
            "s:secret",
        ] {
            assert!(!encoded.contains(secret));
        }

        let mut inactive = status;
        inactive.active = None;
        assert_eq!(
            serde_json::to_value(redact_health_status(&inactive)).expect("inactive JSON")["active"],
            json!({ "present": false, "has_suggestion": false })
        );
    }
}
